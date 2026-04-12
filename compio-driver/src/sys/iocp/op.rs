#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    os::windows::io::AsRawSocket,
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
            SIO_GET_EXTENSION_FUNCTION_POINTER, SO_UPDATE_ACCEPT_CONTEXT,
            SO_UPDATE_CONNECT_CONTEXT, SOCKADDR, SOCKADDR_STORAGE, SOL_SOCKET, WSAID_ACCEPTEX,
            WSAID_CONNECTEX, WSAID_GETACCEPTEXSOCKADDRS, WSAID_WSARECVMSG, WSAIoctl, WSAMSG,
            WSARecv, WSARecvFrom, WSASend, WSASendMsg, WSASendTo, closesocket, setsockopt,
            socklen_t,
        },
        Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile},
        System::{
            IO::{CancelIoEx, DeviceIoControl, OVERLAPPED},
            Pipes::ConnectNamedPipe,
        },
    },
    core::GUID,
};

pub use self::{
    Send as SendZc, SendMsg as SendMsgZc, SendTo as SendToZc, SendToVectored as SendToVectoredZc,
    SendVectored as SendVectoredZc,
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

unsafe impl<D, F> OpCode for Asyncify<F, D>
where
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd);
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S1, S2, D, F> OpCode for AsyncifyFd2<S1, S2, F, D>
where
    S1: std::marker::Sync,
    S2: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd1, &self.fd2);
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, CloseHandle(self.fd.as_fd().as_raw_fd()))? as _,
        ))
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.sys_slice_mut();
        let fd = self.fd.as_fd().as_raw_fd();
        let mut transferred = 0;
        let res = unsafe {
            ReadFile(
                fd,
                slice.ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.as_init();
        let mut transferred = 0;
        let res = unsafe {
            WriteFile(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len().try_into().unwrap_or(u32::MAX),
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<S: AsFd> OpCode for ReadManagedAt<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let mut transferred = 0;
        let slice = self.buffer.sys_slice_mut();
        let res = unsafe {
            ReadFile(
                fd,
                slice.ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        let mut transferred = 0;
        let res = unsafe {
            WriteFile(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len().try_into().unwrap_or(u32::MAX),
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<S: AsFd> OpCode for ReadManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(
        &mut self,
        control: &mut (),
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, FlushFileBuffers(self.fd.as_fd().as_raw_fd()))? as _,
        ))
    }
}

unsafe impl OpCode for CloseSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
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
}

impl<S> Accept<S> {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: S, accept_fd: socket2::Socket) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: [0u8; ACCEPT_BUFFER_SIZE],
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

unsafe impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
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
                self.buffer.sys_slice_mut().ptr() as _,
                0,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                &mut received,
                optr,
            )
        };
        win32_result(res, received)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
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

unsafe impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
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

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Receive data from remote.
///
/// It is only used for socket operations. If you want to read from a pipe, use
/// [`Read`].
pub struct Recv<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct RecvControl {
    pub(crate) slice: SysSlice,
}

impl<T: IoBufMut, S> Recv<T, S> {
    /// Create [`Recv`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoBufMut, S> IntoInner for Recv<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = RecvControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slice = self.buffer.sys_slice_mut();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let mut flags = self.flags as _;
        let mut received = 0;
        let res = unsafe {
            WSARecv(
                fd as _,
                &raw const control.slice as _,
                1,
                &mut received,
                &mut flags,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<S: AsFd> OpCode for RecvManaged<S> {
    type Control = RecvControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct RecvVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    /// Create [`RecvVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = RecvVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let mut flags = self.flags as _;
        let mut received = 0;
        let res = unsafe {
            WSARecv(
                fd as _,
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                &mut received,
                &mut flags,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to remote.
///
/// It is only used for socket operations. If you want to write to a pipe, use
/// [`Write`].
pub struct Send<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct SendControl {
    pub(crate) slice: SysSlice,
}

impl<T: IoBuf, S> Send<T, S> {
    /// Create [`Send`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoBuf, S> IntoInner for Send<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = SendControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slice = self.buffer.sys_slice();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let mut sent = 0;
        let res = unsafe {
            WSASend(
                self.fd.as_fd().as_raw_fd() as _,
                (&raw const control.slice).cast(),
                1,
                &mut sent,
                self.flags as _,
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct SendVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    /// Create [`SendVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = SendVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let mut sent = 0;
        let res = unsafe {
            WSASend(
                self.fd.as_fd().as_raw_fd() as _,
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                &mut sent,
                self.flags as _,
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct RecvFromControl {
    pub(crate) slice: SysSlice,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        let addr = SockAddrStorage::zeroed();
        let addr_len = addr.size_of();
        Self {
            fd,
            buffer,
            addr,
            addr_len,
            flags,
        }
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = (self.addr_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.addr_len) });
        (self.buffer, addr)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = RecvFromControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slice = self.buffer.sys_slice_mut();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let mut flags = self.flags as _;
        let mut received = 0;
        let res = unsafe {
            WSARecvFrom(
                fd as _,
                (&raw const control.slice).cast(),
                1,
                &mut received,
                &mut flags,
                &raw mut self.addr as *mut SOCKADDR,
                &raw mut self.addr_len,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromManaged<S> {
    type Control = RecvFromControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromMulti<S> {
    type Control = RecvFromControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }

    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        result: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        if let Ok(result) = result {
            self.len = *result;
        }
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct RecvFromVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        let addr = SockAddrStorage::zeroed();
        let addr_len = addr.size_of();
        Self {
            fd,
            buffer,
            addr,
            addr_len,
            flags,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = (self.addr_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.addr_len) });
        (self.buffer, addr)
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = RecvFromVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let mut flags = self.flags as _;
        let mut received = 0;
        let res = unsafe {
            WSARecvFrom(
                fd as _,
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                &mut received,
                &mut flags,
                &raw mut self.addr as *mut SOCKADDR,
                &raw mut self.addr_len,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to specified address from buffer.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct SendToControl {
    pub(crate) slice: SysSlice,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
        }
    }
}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = SendToControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slice = self.buffer.sys_slice();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let mut sent = 0;
        let res = unsafe {
            WSASendTo(
                self.fd.as_fd().as_raw_fd() as _,
                (&raw const control.slice).cast(),
                1,
                &mut sent,
                self.flags as _,
                self.addr.as_ptr().cast(),
                self.addr.len(),
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) flags: i32,
}

#[derive(Default)]
pub struct SendToVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = SendToVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let mut sent = 0;
        let res = unsafe {
            WSASendTo(
                self.fd.as_fd().as_raw_fd() as _,
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                &mut sent,
                self.flags as _,
                self.addr.as_ptr().cast(),
                self.addr.len(),
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

static WSA_RECVMSG: OnceLock<LPFN_WSARECVMSG> = OnceLock::new();

/// Receive data and source address with ancillary data into vectored buffer.
pub struct RecvMsg<T: IoVectoredBufMut, C: IoBufMut, S> {
    addr: SockAddrStorage,
    fd: S,
    buffer: T,
    control: C,
    flags: i32,
    name_len: socklen_t,
    control_len: usize,
}

#[derive(Default)]
pub struct RecvMsgControl {
    msg: WSAMSG,
    #[allow(dead_code)]
    slices: Vec<SysSlice>,
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> RecvMsg<T, C, S> {
    /// Create [`RecvMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C, flags: i32) -> Self {
        assert!(
            control.buf_ptr().cast::<CMSGHDR>().is_aligned(),
            "misaligned control message buffer"
        );
        let addr = SockAddrStorage::zeroed();
        Self {
            addr,
            fd,
            buffer,
            control,
            flags,
            name_len: 0,
            control_len: 0,
        }
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> IntoInner for RecvMsg<T, C, S> {
    type Inner = ((T, C), Option<SockAddr>, usize);

    fn into_inner(self) -> Self::Inner {
        let addr = (self.name_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.name_len) });

        ((self.buffer, self.control), addr, self.control_len)
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut();
        ctrl.msg.dwFlags = self.flags as _;
        ctrl.msg.name = &raw mut self.addr as _;
        ctrl.msg.namelen = self.addr.size_of() as _;
        ctrl.msg.lpBuffers = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.dwBufferCount = ctrl.slices.len() as _;
        ctrl.msg.Control = self.control.sys_slice_mut().into_inner();
    }

    unsafe fn operate(
        &mut self,
        control: &mut RecvMsgControl,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let recvmsg_fn = WSA_RECVMSG
            .get_or_try_init(|| get_wsa_fn(self.fd.as_fd().as_raw_fd(), WSAID_WSARECVMSG))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve WSARecvMsg")
            })?;

        let mut received = 0;
        let res = unsafe {
            recvmsg_fn(
                self.fd.as_fd().as_raw_fd() as _,
                &raw mut control.msg,
                &mut received,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.flags = control.msg.dwFlags as i32;
        self.name_len = control.msg.namelen as socklen_t;
        self.control_len = control.msg.Control.len as _;
    }
}

unsafe impl<C: IoBufMut, S: AsFd> OpCode for RecvMsgManaged<C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, res, extra) }
    }
}

unsafe impl<S: AsFd> OpCode for RecvMsgMulti<S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, res, extra) };
        if let Ok(res) = res {
            self.len = *res;
        }
    }
}

/// Send data to specified address accompanied by ancillary data from vectored
/// buffer.
pub struct SendMsg<T: IoVectoredBuf, C: IoBuf, S> {
    fd: S,
    buffer: T,
    control: C,
    addr: Option<SockAddr>,
    flags: i32,
}

#[derive(Default)]
pub struct SendMsgControl {
    msg: WSAMSG,
    #[allow(dead_code)]
    slices: Vec<SysSlice>,
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsg<T, C, S> {
    /// Create [`SendMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C, addr: Option<SockAddr>, flags: i32) -> Self {
        assert!(
            control.buf_len() == 0 || control.buf_ptr().cast::<CMSGHDR>().is_aligned(),
            "misaligned control message buffer"
        );
        Self {
            fd,
            buffer,
            control,
            addr,
            flags,
        }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsg<T, C, S> {
    type Inner = (T, C);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.control)
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
        let control = if self.control.buf_len() == 0 {
            SysSlice::null()
        } else {
            self.control.sys_slice()
        };

        ctrl.msg.lpBuffers = ctrl.slices.as_ptr() as _;
        ctrl.msg.dwBufferCount = ctrl.slices.len() as _;
        ctrl.msg.Control = control.into_inner();
        if let Some(addr) = &self.addr {
            ctrl.msg.name = addr.as_ptr() as _;
            ctrl.msg.namelen = addr.len() as _;
        }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let mut sent = 0;
        let res = unsafe {
            WSASendMsg(
                self.fd.as_fd().as_raw_fd() as _,
                &raw mut control.msg,
                self.flags as _,
                &mut sent,
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Connect to a named pipe.
pub struct ConnectNamedPipe<S> {
    pub(crate) fd: S,
}

impl<S> ConnectNamedPipe<S> {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: S) -> Self {
        Self { fd }
    }
}

unsafe impl<S: AsFd> OpCode for ConnectNamedPipe<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = unsafe { ConnectNamedPipe(self.fd.as_fd().as_raw_fd() as _, optr) };
        win32_result(res, 0)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send a control code to a device.
pub struct DeviceIoControl<S, I: IoBuf, O: IoBufMut> {
    pub(crate) fd: S,
    pub(crate) ioctl_code: u32,
    pub(crate) input_buffer: Option<I>,
    pub(crate) output_buffer: Option<O>,
}

impl<S, I: IoBuf, O: IoBufMut> DeviceIoControl<S, I, O> {
    /// Create [`DeviceIoControl`].
    pub fn new(fd: S, ioctl_code: u32, input_buffer: Option<I>, output_buffer: Option<O>) -> Self {
        Self {
            fd,
            ioctl_code,
            input_buffer,
            output_buffer,
        }
    }
}

impl<S, I: IoBuf, O: IoBufMut> IntoInner for DeviceIoControl<S, I, O> {
    type Inner = (Option<I>, Option<O>);

    fn into_inner(self) -> Self::Inner {
        (self.input_buffer, self.output_buffer)
    }
}

unsafe impl<S: AsFd, I: IoBuf, O: IoBufMut> OpCode for DeviceIoControl<S, I, O> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();

        let input = self
            .input_buffer
            .as_ref()
            .map_or(SysSlice::null(), |x| x.sys_slice());
        let output = self
            .output_buffer
            .as_mut()
            .map_or(SysSlice::null(), |x| x.sys_slice_mut());

        let mut transferred = 0;
        let res = unsafe {
            DeviceIoControl(
                fd,
                self.ioctl_code,
                input.ptr() as _,
                input.len() as _,
                output.ptr() as _,
                output.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}
