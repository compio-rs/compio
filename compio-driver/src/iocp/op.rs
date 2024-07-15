#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    marker::PhantomPinned,
    net::Shutdown,
    os::windows::io::AsRawSocket,
    pin::Pin,
    ptr::{null, null_mut},
    task::Poll,
};

use aligned_array::{Aligned, A8};
use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use socket2::SockAddr;
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE,
            ERROR_IO_PENDING, ERROR_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
            ERROR_PIPE_NOT_CONNECTED,
        },
        Networking::WinSock::{
            closesocket, setsockopt, shutdown, socklen_t, WSAIoctl, WSARecv, WSARecvFrom, WSASend,
            WSASendTo, LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_GETACCEPTEXSOCKADDRS, SD_BOTH,
            SD_RECEIVE, SD_SEND, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKADDR_STORAGE,
            SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSAID_ACCEPTEX,
            WSAID_CONNECTEX, WSAID_GETACCEPTEXSOCKADDRS,
        },
        Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile},
        System::{
            Pipes::ConnectNamedPipe,
            IO::{CancelIoEx, OVERLAPPED},
        },
    },
};

use crate::{op::*, syscall, AsRawFd, OpCode, OpType, RawFd, SharedFd};

#[inline]
fn winapi_result(transferred: u32) -> Poll<io::Result<usize>> {
    let error = unsafe { GetLastError() };
    assert_ne!(error, 0);
    match error {
        ERROR_IO_PENDING => Poll::Pending,
        ERROR_IO_INCOMPLETE
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
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + std::marker::Sync + 'static,
> OpCode for Asyncify<F, D>
{
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // Safety: self won't be moved
        let this = self.get_unchecked_mut();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        this.data = Some(data);
        Poll::Ready(res)
    }
}

impl OpCode for CloseFile {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(BOOL, CloseHandle(self.fd.as_raw_fd()))? as _))
    }
}

impl<T: IoBufMut, S: AsRawFd> OpCode for ReadAt<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let fd = self.fd.as_raw_fd();
        let slice = self.get_unchecked_mut().buffer.as_mut_slice();
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

impl<T: IoBuf, S: AsRawFd> OpCode for WriteAt<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = WriteFile(
            self.fd.as_raw_fd(),
            slice.as_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_raw_fd(), optr)
    }
}

impl<S: AsRawFd> OpCode for Sync<S> {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, FlushFileBuffers(self.fd.as_raw_fd()))? as _
        ))
    }
}

impl<S: AsRawFd> OpCode for ShutdownSocket<S> {
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
            syscall!(SOCKET, shutdown(self.fd.as_raw_fd() as _, how))? as _,
        ))
    }
}

impl OpCode for CloseSocket {
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(SOCKET, closesocket(self.fd.as_raw_fd() as _))? as _
        ))
    }
}

static ACCEPT_EX: OnceLock<LPFN_ACCEPTEX> = OnceLock::new();
static GET_ADDRS: OnceLock<LPFN_GETACCEPTEXSOCKADDRS> = OnceLock::new();

const ACCEPT_ADDR_BUFFER_SIZE: usize = std::mem::size_of::<SOCKADDR_STORAGE>() + 16;
const ACCEPT_BUFFER_SIZE: usize = ACCEPT_ADDR_BUFFER_SIZE * 2;

/// Accept a connection.
pub struct Accept<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) accept_fd: socket2::Socket,
    pub(crate) buffer: Aligned<A8, [u8; ACCEPT_BUFFER_SIZE]>,
    _p: PhantomPinned,
}

impl<S> Accept<S> {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: SharedFd<S>, accept_fd: socket2::Socket) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }
}

impl<S: AsRawFd> Accept<S> {
    /// Update accept context.
    pub fn update_context(&self) -> io::Result<()> {
        let fd = self.fd.as_raw_fd();
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
            .get_or_try_init(|| get_wsa_fn(self.fd.as_raw_fd(), WSAID_GETACCEPTEXSOCKADDRS))?
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
            SockAddr::new(*remote_addr.cast::<SOCKADDR_STORAGE>(), remote_addr_len)
        }))
    }
}

impl<S: AsRawFd> OpCode for Accept<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let accept_fn = ACCEPT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd.as_raw_fd(), WSAID_ACCEPTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve AcceptEx")
            })?;
        let mut received = 0;
        let res = accept_fn(
            self.fd.as_raw_fd() as _,
            self.accept_fd.as_raw_socket() as _,
            self.get_unchecked_mut().buffer.as_mut_ptr() as _,
            0,
            ACCEPT_ADDR_BUFFER_SIZE as _,
            ACCEPT_ADDR_BUFFER_SIZE as _,
            &mut received,
            optr,
        );
        win32_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_raw_fd(), optr)
    }
}

static CONNECT_EX: OnceLock<LPFN_CONNECTEX> = OnceLock::new();

impl<S: AsRawFd> Connect<S> {
    /// Update connect context.
    pub fn update_context(&self) -> io::Result<()> {
        syscall!(
            SOCKET,
            setsockopt(
                self.fd.as_raw_fd() as _,
                SOL_SOCKET,
                SO_UPDATE_CONNECT_CONTEXT,
                null(),
                0,
            )
        )?;
        Ok(())
    }
}

impl<S: AsRawFd> OpCode for Connect<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let connect_fn = CONNECT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd.as_raw_fd(), WSAID_CONNECTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve ConnectEx")
            })?;
        let mut sent = 0;
        let res = connect_fn(
            self.fd.as_raw_fd() as _,
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Receive data from remote.
pub struct Recv<T: IoBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> Recv<T, S> {
    /// Create [`Recv`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
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

impl<T: IoBufMut, S: AsRawFd> OpCode for Recv<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();
        let slice = self.get_unchecked_mut().buffer.as_mut_slice();
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    /// Create [`RecvVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
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

impl<T: IoVectoredBufMut, S: AsRawFd> OpCode for RecvVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_raw_fd();
        let slices = self.get_unchecked_mut().buffer.io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecv(
            fd as _,
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> Send<T, S> {
    /// Create [`Send`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
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

impl<T: IoBuf, S: AsRawFd> OpCode for Send<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = WriteFile(
            self.fd.as_raw_fd(),
            slice.as_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    /// Create [`SendVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
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

impl<T: IoVectoredBuf, S: AsRawFd> OpCode for SendVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slices = self.buffer.io_slices();
        let mut sent = 0;
        let res = WSASend(
            self.fd.as_raw_fd() as _,
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoBufMut, S: AsRawFd> OpCode for RecvFrom<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let fd = this.fd.as_raw_fd();
        let buffer = this.buffer.as_io_slice_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            fd as _,
            &buffer as *const _ as _,
            1,
            &mut received,
            &mut flags,
            &mut this.addr as *mut _ as *mut SOCKADDR,
            &mut this.addr_len,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoVectoredBufMut, S: AsRawFd> OpCode for RecvFromVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let fd = this.fd.as_raw_fd();
        let buffer = this.buffer.io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            fd as _,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut received,
            &mut flags,
            &mut this.addr as *mut _ as *mut SOCKADDR,
            &mut this.addr_len,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: SharedFd<S>, buffer: T, addr: SockAddr) -> Self {
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

impl<T: IoBuf, S: AsRawFd> OpCode for SendTo<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slice();
        let mut sent = 0;
        let res = WSASendTo(
            self.fd.as_raw_fd() as _,
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T, addr: SockAddr) -> Self {
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

impl<T: IoVectoredBuf, S: AsRawFd> OpCode for SendToVectored<T, S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.io_slices();
        let mut sent = 0;
        let res = WSASendTo(
            self.fd.as_raw_fd() as _,
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
        cancel(self.fd.as_raw_fd(), optr)
    }
}

/// Connect a named pipe server.
pub struct ConnectNamedPipe<S> {
    pub(crate) fd: SharedFd<S>,
}

impl<S> ConnectNamedPipe<S> {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: SharedFd<S>) -> Self {
        Self { fd }
    }
}

impl<S: AsRawFd> OpCode for ConnectNamedPipe<S> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = ConnectNamedPipe(self.fd.as_raw_fd() as _, optr);
        win32_result(res, 0)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_raw_fd(), optr)
    }
}
