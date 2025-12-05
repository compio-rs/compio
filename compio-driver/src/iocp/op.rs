#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    marker::PhantomPinned,
    net::Shutdown,
    os::windows::io::AsRawSocket,
    pin::Pin,
    ptr::{null, null_mut, read_unaligned},
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use socket2::{SockAddr, SockAddrStorage};
use windows_sys::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE,
            ERROR_IO_PENDING, ERROR_NETNAME_DELETED, ERROR_NO_DATA, ERROR_NOT_FOUND,
            ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GetLastError,
        },
        Networking::WinSock::{
            CMSGHDR, LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_GETACCEPTEXSOCKADDRS, LPFN_WSARECVMSG,
            SD_BOTH, SD_RECEIVE, SD_SEND, SIO_GET_EXTENSION_FUNCTION_POINTER,
            SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, SOCKADDR, SOCKADDR_STORAGE,
            SOL_SOCKET, WSAID_ACCEPTEX, WSAID_CONNECTEX, WSAID_GETACCEPTEXSOCKADDRS,
            WSAID_WSARECVMSG, WSAIoctl, WSAMSG, WSARecv, WSARecvFrom, WSASend, WSASendMsg,
            WSASendTo, closesocket, setsockopt, shutdown, socklen_t,
        },
        Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile},
        System::{
            IO::{CancelIoEx, DeviceIoControl, OVERLAPPED},
            Pipes::ConnectNamedPipe,
        },
    },
    core::GUID,
};

use crate::{AsFd, AsRawFd, OpCode, OpType, RawFd, op::*, sys_slice::*, syscall};

#[inline]
fn winapi_result(transferred: u32) -> Poll<io::Result<usize>> {
    let error = unsafe { GetLastError() };
    assert_ne!(error, 0);
    match error {
        ERROR_IO_PENDING => Poll::Pending,
        ERROR_IO_INCOMPLETE
        | ERROR_NETNAME_DELETED
        | ERROR_HANDLE_EOF
        | ERROR_BROKEN_PIPE
        | ERROR_PIPE_CONNECTED
        | ERROR_PIPE_NOT_CONNECTED
        | ERROR_NO_DATA => Poll::Ready(Ok(transferred as _)),
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

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let this = unsafe { self.get_unchecked_mut() };
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        this.data = Some(data);
        Poll::Ready(res)
    }
}

impl<
    S,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd<S, F, D>
{
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let this = unsafe { self.get_unchecked_mut() };
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&this.fd);
        this.data = Some(data);
        Poll::Ready(res)
    }
}

impl OpCode for CloseFile {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, CloseHandle(self.fd.as_fd().as_raw_fd()))? as _,
        ))
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_uninit();
        let mut transferred = 0;
        let res = unsafe {
            ReadFile(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = unsafe {
            WriteFile(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

impl<S: AsFd> OpCode for ReadManagedAt<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op).operate(optr) }
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op).cancel(optr) }
    }
}

impl<S: AsFd> OpCode for Sync<S> {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, FlushFileBuffers(self.fd.as_fd().as_raw_fd()))? as _,
        ))
    }
}

impl<S: AsFd> OpCode for ShutdownSocket<S> {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let how = match self.how {
            Shutdown::Write => SD_SEND,
            Shutdown::Read => SD_RECEIVE,
            Shutdown::Both => SD_BOTH,
        };
        Poll::Ready(Ok(
            syscall!(SOCKET, shutdown(self.fd.as_fd().as_raw_fd() as _, how))? as _,
        ))
    }
}

impl OpCode for CloseSocket {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(SOCKET, closesocket(self.fd.as_fd().as_raw_fd() as _))? as _,
        ))
    }
}

static ACCEPT_EX: OnceLock<LPFN_ACCEPTEX> = OnceLock::new();
static GET_ADDRS: OnceLock<LPFN_GETACCEPTEXSOCKADDRS> = OnceLock::new();

const ACCEPT_ADDR_BUFFER_SIZE: usize = std::mem::size_of::<SOCKADDR_STORAGE>() + 16;
const ACCEPT_BUFFER_SIZE: usize = ACCEPT_ADDR_BUFFER_SIZE * 2;

/// Accept a connection.
pub struct Accept<S> {
    pub(crate) fd: S,
    pub(crate) accept_fd: socket2::Socket,
    pub(crate) buffer: [u8; ACCEPT_BUFFER_SIZE],
    _p: PhantomPinned,
}

impl<S> Accept<S> {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: S, accept_fd: socket2::Socket) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: [0u8; ACCEPT_BUFFER_SIZE],
            _p: PhantomPinned,
        }
    }
}

impl<S: AsFd> Accept<S> {
    /// Update accept context.
    pub fn update_context(&self) -> io::Result<()> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(
            SOCKET,
            setsockopt(
                self.accept_fd.as_raw_socket() as _,
                SOL_SOCKET,
                SO_UPDATE_ACCEPT_CONTEXT,
                &fd as *const _ as _,
                std::mem::size_of_val(&fd) as _,
            )
        )?;
        Ok(())
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> io::Result<(socket2::Socket, SockAddr)> {
        let get_addrs_fn = GET_ADDRS
            .get_or_try_init(|| {
                get_wsa_fn(self.fd.as_fd().as_raw_fd(), WSAID_GETACCEPTEXSOCKADDRS)
            })?
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
                ACCEPT_ADDR_BUFFER_SIZE as _,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                &mut local_addr,
                &mut local_addr_len,
                &mut remote_addr,
                &mut remote_addr_len,
            );
        }
        Ok((self.accept_fd, unsafe {
            SockAddr::new(
                // SAFETY: the buffer is large enough to hold the address
                std::mem::transmute::<SOCKADDR_STORAGE, SockAddrStorage>(read_unaligned(
                    remote_addr.cast::<SOCKADDR_STORAGE>(),
                )),
                remote_addr_len,
            )
        }))
    }
}

impl<S: AsFd> OpCode for Accept<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let accept_fn = ACCEPT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd.as_fd().as_raw_fd(), WSAID_ACCEPTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve AcceptEx")
            })?;
        let mut received = 0;
        let res = unsafe {
            accept_fn(
                self.fd.as_fd().as_raw_fd() as _,
                self.accept_fd.as_raw_socket() as _,
                self.get_unchecked_mut().buffer.as_mut_ptr() as _,
                0,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                &mut received,
                optr,
            )
        };
        win32_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

static CONNECT_EX: OnceLock<LPFN_CONNECTEX> = OnceLock::new();

impl<S: AsFd> Connect<S> {
    /// Update connect context.
    pub fn update_context(&self) -> io::Result<()> {
        syscall!(
            SOCKET,
            setsockopt(
                self.fd.as_fd().as_raw_fd() as _,
                SOL_SOCKET,
                SO_UPDATE_CONNECT_CONTEXT,
                null(),
                0,
            )
        )?;
        Ok(())
    }
}

impl<S: AsFd> OpCode for Connect<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let connect_fn = CONNECT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd.as_fd().as_raw_fd(), WSAID_CONNECTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve ConnectEx")
            })?;
        let mut sent = 0;
        let res = unsafe {
            connect_fn(
                self.fd.as_fd().as_raw_fd() as _,
                self.addr.as_ptr().cast(),
                self.addr.len(),
                null(),
                0,
                &mut sent,
                optr,
            )
        };
        win32_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Receive data from remote.
pub struct Recv<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> Recv<T, S> {
    /// Create [`Recv`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut, S> IntoInner for Recv<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<S: AsFd> OpCode for RecvManaged<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op).operate(optr) }
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op).cancel(optr) }
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_uninit();
        let mut transferred = 0;
        let res = unsafe {
            ReadFile(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    /// Create [`RecvVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let slices = unsafe { self.get_unchecked_mut().buffer.sys_slices_mut() };
        let mut flags = 0;
        let mut received = 0;
        let res = unsafe {
            WSARecv(
                fd as _,
                slices.as_ptr() as _,
                slices.len() as _,
                &mut received,
                &mut flags,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> Send<T, S> {
    /// Create [`Send`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf, S> IntoInner for Send<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = unsafe {
            WriteFile(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    /// Create [`SendVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slices = unsafe { self.buffer.sys_slices() };
        let mut sent = 0;
        let res = unsafe {
            WSASend(
                self.fd.as_fd().as_raw_fd() as _,
                slices.as_ptr() as _,
                slices.len() as _,
                &mut sent,
                0,
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T) -> Self {
        let addr = SockAddrStorage::zeroed();
        let addr_len = addr.size_of();
        Self {
            fd,
            buffer,
            addr,
            addr_len,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        let fd = this.fd.as_fd().as_raw_fd();
        let buffer: SysSlice = this.buffer.as_uninit().into();
        let mut flags = 0;
        let mut received = 0;
        let res = unsafe {
            WSARecvFrom(
                fd as _,
                &buffer as *const _ as _,
                1,
                &mut received,
                &mut flags,
                &mut this.addr as *mut _ as *mut SOCKADDR,
                &mut this.addr_len,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        let addr = SockAddrStorage::zeroed();
        let addr_len = addr.size_of();
        Self {
            fd,
            buffer,
            addr,
            addr_len,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        let fd = this.fd.as_fd().as_raw_fd();
        let buffer = unsafe { this.buffer.sys_slices_mut() };
        let mut flags = 0;
        let mut received = 0;
        let res = unsafe {
            WSARecvFrom(
                fd as _,
                buffer.as_ptr() as _,
                buffer.len() as _,
                &mut received,
                &mut flags,
                &mut this.addr as *mut _ as *mut SOCKADDR,
                &mut this.addr_len,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer: SysSlice = self.buffer.as_slice().into();
        let mut sent = 0;
        let res = unsafe {
            WSASendTo(
                self.fd.as_fd().as_raw_fd() as _,
                &buffer as *const _ as _,
                1,
                &mut sent,
                0,
                self.addr.as_ptr().cast(),
                self.addr.len(),
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = unsafe { self.buffer.sys_slices() };
        let mut sent = 0;
        let res = unsafe {
            WSASendTo(
                self.fd.as_fd().as_raw_fd() as _,
                buffer.as_ptr() as _,
                buffer.len() as _,
                &mut sent,
                0,
                self.addr.as_ptr().cast(),
                self.addr.len(),
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

static WSA_RECVMSG: OnceLock<LPFN_WSARECVMSG> = OnceLock::new();

/// Receive data and source address with ancillary data into vectored buffer.
pub struct RecvMsg<T: IoVectoredBufMut, C: IoBufMut, S> {
    msg: WSAMSG,
    addr: SockAddrStorage,
    fd: S,
    buffer: T,
    control: C,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> RecvMsg<T, C, S> {
    /// Create [`RecvMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C) -> Self {
        assert!(
            control.buf_ptr().cast::<CMSGHDR>().is_aligned(),
            "misaligned control message buffer"
        );
        let addr = SockAddrStorage::zeroed();
        Self {
            msg: unsafe { std::mem::zeroed() },
            addr,
            fd,
            buffer,
            control,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> IntoInner for RecvMsg<T, C, S> {
    type Inner = ((T, C), SockAddrStorage, socklen_t, usize);

    fn into_inner(self) -> Self::Inner {
        (
            (self.buffer, self.control),
            self.addr,
            self.msg.namelen,
            self.msg.Control.len as _,
        )
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let recvmsg_fn = WSA_RECVMSG
            .get_or_try_init(|| get_wsa_fn(self.fd.as_fd().as_raw_fd(), WSAID_WSARECVMSG))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve WSARecvMsg")
            })?;

        let this = unsafe { self.get_unchecked_mut() };

        let mut slices = unsafe { this.buffer.sys_slices_mut() };
        let sys_slice: SysSlice = this.control.as_uninit().into();
        this.msg.name = &mut this.addr as *mut _ as _;
        this.msg.namelen = std::mem::size_of::<SOCKADDR_STORAGE>() as _;
        this.msg.lpBuffers = slices.as_mut_ptr() as _;
        this.msg.dwBufferCount = slices.len() as _;
        this.msg.Control = sys_slice.into_inner();

        let mut received = 0;
        let res = unsafe {
            recvmsg_fn(
                this.fd.as_fd().as_raw_fd() as _,
                &mut this.msg,
                &mut received,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to specified address accompanied by ancillary data from vectored
/// buffer.
pub struct SendMsg<T: IoVectoredBuf, C: IoBuf, S> {
    fd: S,
    buffer: T,
    control: C,
    addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsg<T, C, S> {
    /// Create [`SendMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C, addr: SockAddr) -> Self {
        assert!(
            control.buf_ptr().cast::<CMSGHDR>().is_aligned(),
            "misaligned control message buffer"
        );
        Self {
            fd,
            buffer,
            control,
            addr,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsg<T, C, S> {
    type Inner = (T, C);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.control)
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };

        let slices = unsafe { this.buffer.sys_slices() };
        let control: SysSlice = this.control.as_slice().into();
        let msg = WSAMSG {
            name: this.addr.as_ptr() as _,
            namelen: this.addr.len(),
            lpBuffers: slices.as_ptr() as _,
            dwBufferCount: slices.len() as _,
            Control: control.into_inner(),
            dwFlags: 0,
        };

        let mut sent = 0;
        let res = unsafe {
            WSASendMsg(
                this.fd.as_fd().as_raw_fd() as _,
                &msg,
                0,
                &mut sent,
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Connect a named pipe server.
pub struct ConnectNamedPipe<S> {
    pub(crate) fd: S,
}

impl<S> ConnectNamedPipe<S> {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: S) -> Self {
        Self { fd }
    }
}

impl<S: AsFd> OpCode for ConnectNamedPipe<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = unsafe { ConnectNamedPipe(self.fd.as_fd().as_raw_fd() as _, optr) };
        win32_result(res, 0)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send a control code to a device.
pub struct DeviceIoControl<S, I: IoBuf, O: IoBufMut> {
    pub(crate) fd: S,
    pub(crate) ioctl_code: u32,
    pub(crate) input_buffer: Option<I>,
    pub(crate) output_buffer: Option<O>,
    _p: PhantomPinned,
}

impl<S, I: IoBuf, O: IoBufMut> DeviceIoControl<S, I, O> {
    /// Create [`DeviceIoControl`].
    pub fn new(fd: S, ioctl_code: u32, input_buffer: Option<I>, output_buffer: Option<O>) -> Self {
        Self {
            fd,
            ioctl_code,
            input_buffer,
            output_buffer,
            _p: PhantomPinned,
        }
    }
}

impl<S, I: IoBuf, O: IoBufMut> IntoInner for DeviceIoControl<S, I, O> {
    type Inner = (Option<I>, Option<O>);

    fn into_inner(self) -> Self::Inner {
        (self.input_buffer, self.output_buffer)
    }
}

impl<S: AsFd, I: IoBuf, O: IoBufMut> OpCode for DeviceIoControl<S, I, O> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        let fd = this.fd.as_fd().as_raw_fd();
        let input_len = this.input_buffer.as_ref().map_or(0, |x| x.buf_len());
        let input_ptr = this
            .input_buffer
            .as_ref()
            .map_or(std::ptr::null(), |x| x.buf_ptr());
        let output_len = this.output_buffer.as_ref().map_or(0, |x| x.buf_len());
        let output_ptr = this
            .output_buffer
            .as_mut()
            .map_or(std::ptr::null_mut(), |x| x.buf_mut_ptr());
        let mut transferred = 0;
        let res = unsafe {
            DeviceIoControl(
                fd,
                this.ioctl_code,
                input_ptr as _,
                input_len as _,
                output_ptr as _,
                output_len as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}
