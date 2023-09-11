#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    ptr::{null, null_mut},
    task::Poll,
};

#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use socket2::SockAddr;
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{
            GetLastError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA,
            ERROR_PIPE_CONNECTED,
        },
        Networking::WinSock::{
            setsockopt, socklen_t, WSAIoctl, WSARecv, WSARecvFrom, WSASend, WSASendTo,
            LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_GETACCEPTEXSOCKADDRS,
            SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKADDR_STORAGE, SOL_SOCKET,
            SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSAID_ACCEPTEX, WSAID_CONNECTEX,
            WSAID_GETACCEPTEXSOCKADDRS,
        },
        Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile},
        System::{Pipes, IO::OVERLAPPED},
    },
};

use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IntoInner, IoBuf, IoBufMut},
    driver::{registered_fd::FDRegistry, Driver, OpCode, RawFd, RegisteredFd},
    op::*,
};

unsafe fn winapi_result(transferred: u32) -> Poll<io::Result<usize>> {
    let error = GetLastError();
    assert_ne!(error, 0);
    match error {
        ERROR_IO_PENDING => Poll::Pending,
        ERROR_IO_INCOMPLETE | ERROR_HANDLE_EOF | ERROR_PIPE_CONNECTED | ERROR_NO_DATA => {
            Poll::Ready(Ok(transferred as _))
        }
        _ => Poll::Ready(Err(io::Error::from_raw_os_error(error as _))),
    }
}

unsafe fn win32_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res == 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

// read, write, send and recv functions may return immediately, indicate that
// the task is completed, but the overlapped result is also posted to the IOCP.
// To make our driver easy, simply return Pending and query the result later.

unsafe fn win32_pending_result(res: i32) -> Poll<io::Result<usize>> {
    if res == 0 {
        winapi_result(0)
    } else {
        Poll::Pending
    }
}

unsafe fn winsock_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res != 0 {
        winapi_result(transferred)
    } else {
        Poll::Pending
    }
}

unsafe fn get_wsa_fn<F>(handle: RawFd, fguid: GUID) -> io::Result<Option<F>> {
    let mut fptr = None;
    let mut returned = 0;
    let res = WSAIoctl(
        handle as _,
        SIO_GET_EXTENSION_FUNCTION_POINTER,
        std::ptr::addr_of!(fguid).cast(),
        std::mem::size_of_val(&fguid) as _,
        std::ptr::addr_of_mut!(fptr).cast(),
        std::mem::size_of::<F>() as _,
        &mut returned,
        null_mut(),
        None,
    );
    if res == 0 {
        Ok(fptr)
    } else {
        Err(io::Error::last_os_error())
    }
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            #[cfg(target_pointer_width = "64")]
            {
                overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
            }
        }
        let slice = self.buffer.as_uninit_slice();
        let res = ReadFile(
            fd_registry.get_raw_fd(self.fd) as _,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            null_mut(),
            optr,
        );
        win32_pending_result(res)
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            #[cfg(target_pointer_width = "64")]
            {
                overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
            }
        }
        let slice = self.buffer.as_slice();
        let res = WriteFile(
            fd_registry.get_raw_fd(self.fd) as _,
            slice.as_ptr() as _,
            slice.len() as _,
            null_mut(),
            optr,
        );
        win32_pending_result(res)
    }
}

impl OpCode for Sync {
    unsafe fn operate(
        &mut self,
        _optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        let res = FlushFileBuffers(fd_registry.get_raw_fd(self.fd) as _);
        win32_result(res, 0)
    }
}

static ACCEPT_EX: OnceLock<LPFN_ACCEPTEX> = OnceLock::new();
static GET_ADDRS: OnceLock<LPFN_GETACCEPTEXSOCKADDRS> = OnceLock::new();

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: RegisteredFd,
    pub(crate) accept_fd: RegisteredFd,
    pub(crate) buffer: SOCKADDR_STORAGE,
}

impl Accept {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: RegisteredFd, accept_fd: RegisteredFd) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: unsafe { std::mem::zeroed() },
        }
    }

    /// Update accept context.
    pub fn update_context(&self, raw_fd: RawFd, raw_accept_fd: RawFd) -> io::Result<()> {
        let res = unsafe {
            setsockopt(
                raw_accept_fd as _,
                SOL_SOCKET,
                SO_UPDATE_ACCEPT_CONTEXT,
                raw_fd as *const _ as _,
                std::mem::size_of_val(&raw_fd) as _,
            )
        };
        if res != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self, raw_accept_fd: RawFd) -> io::Result<SockAddr> {
        let get_addrs_fn = GET_ADDRS
            .get_or_try_init(|| unsafe { get_wsa_fn(raw_accept_fd, WSAID_GETACCEPTEXSOCKADDRS) })?
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
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        let raw_fd = fd_registry.get_raw_fd(self.fd);
        let raw_accept_fd = fd_registry.get_raw_fd(self.accept_fd);

        let accept_fn = ACCEPT_EX
            .get_or_try_init(|| get_wsa_fn(raw_fd, WSAID_ACCEPTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve AcceptEx")
            })?;
        let mut received = 0;
        let res = accept_fn(
            raw_fd as _,
            raw_accept_fd as _,
            &mut self.buffer as *mut _ as *mut _,
            0,
            0,
            std::mem::size_of_val(&self.buffer) as _,
            &mut received,
            optr,
        );
        win32_result(res, received)
    }
}

static CONNECT_EX: OnceLock<LPFN_CONNECTEX> = OnceLock::new();

impl Connect {
    /// Update connect context.
    pub fn update_context(&self, fd: RawFd) -> io::Result<()> {
        let res = unsafe { setsockopt(fd as _, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, null(), 0) };
        if res != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl OpCode for Connect {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        let raw_fd = fd_registry.get_raw_fd(self.fd);
        let connect_fn = CONNECT_EX
            .get_or_try_init(|| get_wsa_fn(raw_fd, WSAID_CONNECTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve ConnectEx")
            })?;
        let mut sent = 0;
        let res = connect_fn(
            raw_fd as _,
            self.addr.as_ptr(),
            self.addr.len(),
            null(),
            0,
            &mut sent,
            optr,
        );
        win32_result(res, sent)
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvImpl<T> {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        self.slices = self.buffer.as_io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecv(
            fd_registry.get_raw_fd(self.fd) as _,
            self.slices.as_ptr() as _,
            self.slices.len() as _,
            &mut received,
            &mut flags,
            optr,
            None,
        );
        winsock_result(res, received)
    }
}

impl<T: AsIoSlices> OpCode for SendImpl<T> {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        self.slices = self.buffer.as_io_slices();
        let mut sent = 0;
        let res = WSASend(
            fd_registry.get_raw_fd(self.fd) as _,
            self.slices.as_ptr() as _,
            self.slices.len() as _,
            &mut sent,
            0,
            optr,
            None,
        );
        winsock_result(res, sent)
    }
}

/// Receive data and source address.
pub struct RecvFromImpl<T: AsIoSlicesMut> {
    pub(crate) fd: RegisteredFd,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
}

impl<T: AsIoSlicesMut> RecvFromImpl<T> {
    /// Create [`RecvFrom`] or [`RecvFromVectored`].
    pub fn new(fd: RegisteredFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
        }
    }
}

impl<T: AsIoSlicesMut> IntoInner for RecvFromImpl<T> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvFromImpl<T> {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            fd_registry.get_raw_fd(self.fd) as _,
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
}

/// Send data to specified address.
pub struct SendToImpl<T: AsIoSlices> {
    pub(crate) fd: RegisteredFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
}

impl<T: AsIoSlices> SendToImpl<T> {
    /// Create [`SendTo`] or [`SendToVectored`].
    pub fn new(fd: RegisteredFd, buffer: T::Inner, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            addr,
        }
    }
}

impl<T: AsIoSlices> IntoInner for SendToImpl<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: AsIoSlices> OpCode for SendToImpl<T> {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slices();
        let mut sent = 0;
        let res = WSASendTo(
            fd_registry.get_raw_fd(self.fd) as _,
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
}

/// Connect a named pipe server.
pub struct ConnectNamedPipe {
    pub(crate) fd: RegisteredFd,
}

impl ConnectNamedPipe {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: RegisteredFd) -> Self {
        Self { fd }
    }
}

impl OpCode for ConnectNamedPipe {
    unsafe fn operate(
        &mut self,
        optr: *mut OVERLAPPED,
        fd_registry: &Driver,
    ) -> Poll<io::Result<usize>> {
        let res = Pipes::ConnectNamedPipe(fd_registry.get_raw_fd(self.fd) as _, optr);
        win32_result(res, 0)
    }
}
