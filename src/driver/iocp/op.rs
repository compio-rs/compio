use crate::{
    buf::{IoBuf, IoBufMut},
    driver::{OpCode, RawFd},
    op::*,
};
use once_cell::sync::OnceCell as OnceLock;
use socket2::SockAddr;
use std::{
    io::{self, IoSlice, IoSliceMut},
    ptr::{null, null_mut},
    task::Poll,
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{
            GetLastError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA,
            ERROR_PIPE_CONNECTED,
        },
        Networking::WinSock::{
            WSAIoctl, WSARecv, WSARecvFrom, WSASend, WSASendTo, LPFN_ACCEPTEX, LPFN_CONNECTEX,
            LPFN_GETACCEPTEXSOCKADDRS, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR,
            SOCKADDR_STORAGE, WSAID_ACCEPTEX, WSAID_CONNECTEX, WSAID_GETACCEPTEXSOCKADDRS,
        },
        Storage::FileSystem::{ReadFile, WriteFile},
        System::IO::OVERLAPPED,
    },
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

unsafe fn winsock_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res != 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
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
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let mut read = 0;
        let slice = self.buffer.as_uninit_slice();
        let res = ReadFile(
            self.fd as _,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            &mut read,
            optr,
        );
        win32_result(res, read)
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let mut written = 0;
        let slice = self.buffer.as_slice();
        let res = WriteFile(
            self.fd as _,
            slice.as_ptr() as _,
            slice.len() as _,
            &mut written,
            optr,
        );
        win32_result(res, written)
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

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> io::Result<SockAddr> {
        let get_addrs_fn = GET_ADDRS
            .get_or_try_init(|| unsafe { get_wsa_fn(self.fd, WSAID_GETACCEPTEXSOCKADDRS) })?;
        let mut local_addr: *mut SOCKADDR = null_mut();
        let mut local_addr_len = 0;
        let mut remote_addr: *mut SOCKADDR = null_mut();
        let mut remote_addr_len = 0;
        unsafe {
            (get_addrs_fn.unwrap())(
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
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let accept_fn = ACCEPT_EX.get_or_try_init(|| get_wsa_fn(self.fd, WSAID_ACCEPTEX))?;
        let mut received = 0;
        let res = accept_fn.unwrap()(
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
}

static CONNECT_EX: OnceLock<LPFN_CONNECTEX> = OnceLock::new();

impl OpCode for Connect {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let connect_fn = CONNECT_EX.get_or_try_init(|| get_wsa_fn(self.fd, WSAID_CONNECTEX))?;
        let mut sent = 0;
        let res = connect_fn.unwrap()(
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
}

impl<T: IoBufMut> OpCode for Recv<T> {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = IoSliceMut::new(unsafe {
            &mut *(self.buffer.as_uninit_slice() as *mut _ as *mut [u8])
        });
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecv(
            self.fd as _,
            &buffer as *const _ as _,
            1,
            &mut received,
            &mut flags,
            optr,
            None,
        );
        winsock_result(res, received)
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = IoSlice::new(self.buffer.as_slice());
        let mut sent = 0;
        let res = WSASend(
            self.fd as _,
            &buffer as *const _ as _,
            1,
            &mut sent,
            0,
            optr,
            None,
        );
        winsock_result(res, sent)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
        }
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = IoSliceMut::new(unsafe {
            &mut *(self.buffer.as_uninit_slice() as *mut _ as *mut [u8])
        });
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
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = IoSlice::new(self.buffer.as_slice());
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
}
