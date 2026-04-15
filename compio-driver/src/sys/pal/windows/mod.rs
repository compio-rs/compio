use std::{fmt, io, ptr, task::Poll};

pub use windows_sys::Win32::Networking::WinSock::CMSGHDR as CmsgHeader;
use windows_sys::{
    Win32::{
        Foundation::{
            ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING,
            ERROR_NETNAME_DELETED, ERROR_NO_DATA, ERROR_NOT_FOUND, ERROR_PIPE_CONNECTED,
            ERROR_PIPE_NOT_CONNECTED, GetLastError,
        },
        Networking::WinSock::{SIO_GET_EXTENSION_FUNCTION_POINTER, WSAIoctl},
        System::IO::{CancelIoEx, OVERLAPPED},
    },
    core::GUID,
};

use crate::syscall;

mod_use::mod_use![fd];

/// The overlapped struct we actually used for IOCP.
#[repr(C)]
pub struct Overlapped {
    /// The base [`OVERLAPPED`].
    pub base: OVERLAPPED,
    /// The unique ID of created driver.
    pub driver: RawFd,
}

impl fmt::Debug for Overlapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlapped")
            .field("base", &"OVERLAPPED")
            .field("driver", &self.driver)
            .finish()
    }
}

impl Overlapped {
    pub(crate) fn new(driver: RawFd) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            driver,
        }
    }
}

// SAFETY: neither field of `OVERLAPPED` is used
unsafe impl Send for Overlapped {}
unsafe impl Sync for Overlapped {}

#[inline]
pub fn winapi_result(transferred: u32) -> Poll<io::Result<usize>> {
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
pub fn win32_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res == 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

#[inline]
pub fn winsock_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res != 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

#[inline]
pub fn cancel(handle: RawFd, optr: *mut OVERLAPPED) -> io::Result<()> {
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

pub fn get_wsa_fn<F>(handle: RawFd, fguid: GUID) -> io::Result<Option<F>> {
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
            ptr::null_mut(),
            None,
        )
    )?;
    Ok(fptr)
}
