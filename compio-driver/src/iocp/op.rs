#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    pin::Pin,
    ptr::{null, null_mut},
    task::Poll,
};

use compio_buf::{IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use socket2::SockAddr;
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{
            GetLastError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NOT_FOUND,
            ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
        },
        Networking::WinSock::{
            setsockopt, socklen_t, WSAIoctl, WSARecv, WSARecvFrom, WSASend, WSASendTo,
            LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_GETACCEPTEXSOCKADDRS,
            SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKADDR_STORAGE, SOL_SOCKET,
            SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSAID_ACCEPTEX, WSAID_CONNECTEX,
            WSAID_GETACCEPTEXSOCKADDRS,
        },
        Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile},
        System::{
            Pipes::ConnectNamedPipe,
            IO::{CancelIoEx, OVERLAPPED},
        },
    },
};

use crate::{op::*, syscall, OpCode, RawFd};

#[inline]
fn winapi_result(transferred: u32) -> Poll<io::Result<usize>> {
    let error = unsafe { GetLastError() };
    assert_ne!(error, 0);
    match error {
        ERROR_IO_PENDING => Poll::Pending,
        ERROR_IO_INCOMPLETE | ERROR_HANDLE_EOF | ERROR_PIPE_CONNECTED | ERROR_NO_DATA => {
            Poll::Ready(Ok(transferred as _))
        }
        _ => Poll::Ready(Err(io::Error::from_raw_os_error(error as _))),
    }
}

#[inline]
fn win32_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res == 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

#[inline]
fn winsock_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res != 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

#[inline]
fn cancel(handle: RawFd, optr: *mut OVERLAPPED) -> io::Result<()> {
    match syscall!(BOOL, CancelIoEx(handle as _, optr)) {
        Ok(_) => Ok(()),
        Err(e) => {
            if e.raw_os_error() == Some(ERROR_NOT_FOUND as _) {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

fn get_wsa_fn<F>(handle: RawFd, fguid: GUID) -> io::Result<Option<F>> {
    let mut fptr = None;
    let mut returned = 0;
    syscall!(
        SOCKET,
        WSAIoctl(
            handle as _,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            std::ptr::addr_of!(fguid).cast(),
            std::mem::size_of_val(&fguid) as _,
            std::ptr::addr_of_mut!(fptr).cast(),
            std::mem::size_of::<F>() as _,
            &mut returned,
            null_mut(),
            None,
        )
    )?;
    Ok(fptr)
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            #[cfg(target_pointer_width = "64")]
            {
                overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
            }
        }
        let fd = self.fd as _;
        let slice = self.buffer.as_uninit_slice();
        let mut transferred = 0;
        let res = ReadFile(
            fd,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            #[cfg(target_pointer_width = "64")]
            {
                overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
            }
        }
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = WriteFile(
            self.fd as _,
            slice.as_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

impl OpCode for Sync {
    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = FlushFileBuffers(self.fd as _);
        win32_result(res, 0)
    }

    unsafe fn cancel(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> io::Result<()> {
        Ok(())
    }
}

static ACCEPT_EX: OnceLock<LPFN_ACCEPTEX> = OnceLock::new();
static GET_ADDRS: OnceLock<LPFN_GETACCEPTEXSOCKADDRS> = OnceLock::new();

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: RawFd,
    pub(crate) accept_fd: RawFd,
    pub(crate) buffer: SOCKADDR_STORAGE,
}

impl Accept {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: RawFd, accept_fd: RawFd) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: unsafe { std::mem::zeroed() },
        }
    }

    /// Update accept context.
    pub fn update_context(&self) -> io::Result<()> {
        syscall!(
            SOCKET,
            setsockopt(
                self.accept_fd as _,
                SOL_SOCKET,
                SO_UPDATE_ACCEPT_CONTEXT,
                &self.fd as *const _ as _,
                std::mem::size_of_val(&self.fd) as _,
            )
        )?;
        Ok(())
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> io::Result<SockAddr> {
        let get_addrs_fn = GET_ADDRS
            .get_or_try_init(|| get_wsa_fn(self.fd, WSAID_GETACCEPTEXSOCKADDRS))?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "cannot retrieve GetAcceptExSockAddrs",
                )
            })?;
        let mut local_addr: *mut SOCKADDR = null_mut();
        let mut local_addr_len = 0;
        let mut remote_addr: *mut SOCKADDR = null_mut();
        let mut remote_addr_len = 0;
        unsafe {
            get_addrs_fn(
                &self.buffer as *const _ as *const _,
                0,
                0,
                std::mem::size_of_val(&self.buffer) as _,
                &mut local_addr,
                &mut local_addr_len,
                &mut remote_addr,
                &mut remote_addr_len,
            );
        }
        Ok(unsafe { SockAddr::new(*remote_addr.cast::<SOCKADDR_STORAGE>(), remote_addr_len) })
    }
}

impl OpCode for Accept {
    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let accept_fn = ACCEPT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd, WSAID_ACCEPTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve AcceptEx")
            })?;
        let mut received = 0;
        let res = accept_fn(
            self.fd as _,
            self.accept_fd as _,
            &mut self.buffer as *mut _ as *mut _,
            0,
            0,
            std::mem::size_of_val(&self.buffer) as _,
            &mut received,
            optr,
        );
        win32_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

static CONNECT_EX: OnceLock<LPFN_CONNECTEX> = OnceLock::new();

impl Connect {
    /// Update connect context.
    pub fn update_context(&self) -> io::Result<()> {
        syscall!(
            SOCKET,
            setsockopt(
                self.fd as _,
                SOL_SOCKET,
                SO_UPDATE_CONNECT_CONTEXT,
                null(),
                0,
            )
        )?;
        Ok(())
    }
}

impl OpCode for Connect {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let connect_fn = CONNECT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd, WSAID_CONNECTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve ConnectEx")
            })?;
        let mut sent = 0;
        let res = connect_fn(
            self.fd as _,
            self.addr.as_ptr(),
            self.addr.len(),
            null(),
            0,
            &mut sent,
            optr,
        );
        win32_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data from remote.
pub struct Recv<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
}

impl<T: IoBufMut> Recv<T> {
    /// Create [`Recv`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoBufMut> IntoInner for Recv<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBufMut> OpCode for Recv<T> {
    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd as _;
        let slice = self.buffer.as_uninit_slice();
        let mut transferred = 0;
        let res = ReadFile(
            fd,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
}

impl<T: IoVectoredBufMut> RecvVectored<T> {
    /// Create [`RecvVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slices = self.buffer.as_io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecv(
            self.fd as _,
            slices.as_ptr() as _,
            slices.len() as _,
            &mut received,
            &mut flags,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
}

impl<T: IoBuf> Send<T> {
    /// Create [`Send`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoBuf> IntoInner for Send<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = WriteFile(
            self.fd as _,
            slice.as_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
}

impl<T: IoVectoredBuf> SendVectored<T> {
    /// Create [`SendVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoVectoredBuf> IntoInner for SendVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slices = self.buffer.as_io_slices();
        let mut sent = 0;
        let res = WSASend(
            self.fd as _,
            slices.as_ptr() as _,
            slices.len() as _,
            &mut sent,
            0,
            optr,
            None,
        );
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
        }
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slice_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            self.fd as _,
            &buffer as *const _ as _,
            1,
            &mut received,
            &mut flags,
            &mut self.addr as *mut _ as *mut SOCKADDR,
            &mut self.addr_len,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
}

impl<T: IoVectoredBufMut> RecvFromVectored<T> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvFromVectored<T> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            self.fd as _,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut received,
            &mut flags,
            &mut self.addr as *mut _ as *mut SOCKADDR,
            &mut self.addr_len,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self { fd, buffer, addr }
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slice();
        let mut sent = 0;
        let res = WSASendTo(
            self.fd as _,
            &buffer as *const _ as _,
            1,
            &mut sent,
            0,
            self.addr.as_ptr(),
            self.addr.len(),
            optr,
            None,
        );
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
}

impl<T: IoVectoredBuf> SendToVectored<T> {
    /// Create [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self { fd, buffer, addr }
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slices();
        let mut sent = 0;
        let res = WSASendTo(
            self.fd as _,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut sent,
            0,
            self.addr.as_ptr(),
            self.addr.len(),
            optr,
            None,
        );
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Connect a named pipe server.
pub struct ConnectNamedPipe {
    pub(crate) fd: RawFd,
}

impl ConnectNamedPipe {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl OpCode for ConnectNamedPipe {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = ConnectNamedPipe(self.fd as _, optr);
        win32_result(res, 0)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}
