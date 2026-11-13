use std::{ffi::c_void, fmt, io, task::Poll};

use socket2::socklen_t;
pub use windows_sys::Win32::Networking::WinSock::CMSGHDR as CmsgHeader;
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING,
        ERROR_MORE_DATA, ERROR_NETNAME_DELETED, ERROR_NO_DATA, ERROR_NOT_FOUND,
        ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GetLastError,
    },
    Networking::WinSock::{
        LPWSAOVERLAPPED_COMPLETION_ROUTINE, SOCKADDR, SOCKET, WSAECONNRESET, WSAGetLastError,
        WSAGetOverlappedResult, WSAMSG,
    },
    System::IO::{CancelIoEx, OVERLAPPED},
};

use crate::{SharedFd, syscall};

mod_use::mod_use![fd];

pub mod reexport {
    pub use super::Overlapped;
}

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

/// Translate a Win32 error code to the error code reported by the equivalent
/// Winsock call.
#[inline]
fn win32_to_wsa(error: u32) -> u32 {
    match error {
        ERROR_NETNAME_DELETED => WSAECONNRESET as _,
        _ => error,
    }
}

#[inline]
fn winapi_poll_result(transferred: u32, error: u32) -> Poll<io::Result<usize>> {
    assert_ne!(error, 0);
    match error {
        ERROR_IO_PENDING => Poll::Pending,
        ERROR_IO_INCOMPLETE
        | ERROR_HANDLE_EOF
        | ERROR_BROKEN_PIPE
        | ERROR_PIPE_CONNECTED
        | ERROR_PIPE_NOT_CONNECTED
        | ERROR_NO_DATA
        | ERROR_MORE_DATA => Poll::Ready(Ok(transferred as _)),
        _ => Poll::Ready(Err(io::Error::from_raw_os_error(win32_to_wsa(error) as _))),
    }
}

#[inline]
pub fn winapi_result(transferred: u32, error: u32) -> io::Result<usize> {
    match winapi_poll_result(transferred, error) {
        Poll::Ready(res) => res,
        Poll::Pending => Err(io::Error::from_raw_os_error(ERROR_IO_PENDING as _)),
    }
}

/// Convert the result of a Win32 call to a driver result.
///
/// `ERROR_IO_PENDING` becomes [`Poll::Pending`], the "operation completed but
/// with a trailing condition" errors become [`Poll::Ready`] with the
/// transferred bytes, and anything else becomes an error.
#[inline]
pub fn win32_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res == 0 {
        winapi_poll_result(transferred, unsafe { GetLastError() })
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

/// Same as [`win32_result`], but reads the error from `WSAGetLastError`.
#[inline]
pub fn winsock_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res != 0 {
        winapi_poll_result(transferred, unsafe { WSAGetLastError() as _ })
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

/// Resolve the error of a failed overlapped socket operation to the Winsock
/// error code.
///
/// A completion packet carries the NTSTATUS of the operation, which the driver
/// converts to a Win32 error code. For a socket that is not the code a Winsock
/// call reports: a refused connection is `ERROR_CONNECTION_REFUSED` instead of
/// `WSAECONNREFUSED`, and [`io::Error::kind`] does not recognize it.
/// `WSAGetOverlappedResult` asks the provider of the socket for its own error.
///
/// A cancelled operation keeps `ERROR_OPERATION_ABORTED`, the code
/// cancellation is detected by.
///
/// # Safety
///
/// `optr` must point to the overlapped of a completed operation on `socket`.
pub unsafe fn winsock_completion_error(
    socket: RawFd,
    optr: *const OVERLAPPED,
    error: io::Error,
) -> io::Error {
    if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED as _) {
        return error;
    }
    let mut transferred = 0;
    let mut flags = 0;
    let res = unsafe { WSAGetOverlappedResult(socket as _, optr, &mut transferred, 0, &mut flags) };
    if res == 0 {
        io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
    } else {
        error
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

/// Per-socket state needed by the asynchronous operations.
///
/// The winsock extensions (`AcceptEx`, `ConnectEx`, ...) are resolved per
/// socket: the pointers returned by `SIO_GET_EXTENSION_FUNCTION_POINTER` belong
/// to the provider serving that socket, and sharing them with another socket
/// crashes when a layered service provider is installed.
#[allow(unused_variables, clippy::too_many_arguments)]
pub trait AsSocketState: AsFd {
    /// `AcceptEx`.
    ///
    /// # Safety
    ///
    /// `output_buffer` must be large enough for both addresses,
    /// `bytes_received` must be valid for writes, and `overlapped` must
    /// outlive the operation.
    unsafe fn wsa_accept_ex(
        &self,
        accept_socket: SOCKET,
        output_buffer: *mut c_void,
        receive_data_length: u32,
        local_address_length: u32,
        remote_address_length: u32,
        bytes_received: *mut u32,
        overlapped: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(unsupported()))
    }

    /// `GetAcceptExSockaddrs`.
    ///
    /// # Safety
    ///
    /// `output_buffer` must hold the addresses written by
    /// [`AsSocketState::wsa_accept_ex`], and the output pointers must be valid
    /// for writes.
    unsafe fn wsa_get_accept_ex_sockaddrs(
        &self,
        output_buffer: *const c_void,
        receive_data_length: u32,
        local_address_length: u32,
        remote_address_length: u32,
        local_sockaddr: *mut *mut SOCKADDR,
        local_sockaddr_length: *mut socklen_t,
        remote_sockaddr: *mut *mut SOCKADDR,
        remote_sockaddr_length: *mut socklen_t,
    ) -> io::Result<()> {
        Err(unsupported())
    }

    /// `ConnectEx`.
    ///
    /// # Safety
    ///
    /// `name` must be valid for `name_length` bytes, `bytes_sent` must be valid
    /// for writes, and `overlapped` must outlive the operation.
    unsafe fn wsa_connect_ex(
        &self,
        name: *const SOCKADDR,
        name_length: socklen_t,
        send_buffer: *const c_void,
        send_data_length: u32,
        bytes_sent: *mut u32,
        overlapped: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(unsupported()))
    }

    /// `DisconnectEx`.
    ///
    /// # Safety
    ///
    /// `overlapped` must outlive the operation.
    unsafe fn wsa_disconnect_ex(
        &self,
        overlapped: *mut OVERLAPPED,
        flags: u32,
        reserved: u32,
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(unsupported()))
    }

    /// `WSARecvMsg`.
    ///
    /// # Safety
    ///
    /// `msg` must describe buffers that outlive the operation, `bytes_received`
    /// must be valid for writes, and `overlapped` must outlive it too.
    unsafe fn wsa_recv_msg(
        &self,
        msg: *mut WSAMSG,
        bytes_received: *mut u32,
        overlapped: *mut OVERLAPPED,
        completion_routine: LPWSAOVERLAPPED_COMPLETION_ROUTINE,
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(unsupported()))
    }
}

impl<T: AsSocketState> AsSocketState for SharedFd<T> {
    unsafe fn wsa_accept_ex(
        &self,
        accept_socket: SOCKET,
        output_buffer: *mut c_void,
        receive_data_length: u32,
        local_address_length: u32,
        remote_address_length: u32,
        bytes_received: *mut u32,
        overlapped: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe {
            (**self).wsa_accept_ex(
                accept_socket,
                output_buffer,
                receive_data_length,
                local_address_length,
                remote_address_length,
                bytes_received,
                overlapped,
            )
        }
    }

    unsafe fn wsa_get_accept_ex_sockaddrs(
        &self,
        output_buffer: *const c_void,
        receive_data_length: u32,
        local_address_length: u32,
        remote_address_length: u32,
        local_sockaddr: *mut *mut SOCKADDR,
        local_sockaddr_length: *mut socklen_t,
        remote_sockaddr: *mut *mut SOCKADDR,
        remote_sockaddr_length: *mut socklen_t,
    ) -> io::Result<()> {
        unsafe {
            (**self).wsa_get_accept_ex_sockaddrs(
                output_buffer,
                receive_data_length,
                local_address_length,
                remote_address_length,
                local_sockaddr,
                local_sockaddr_length,
                remote_sockaddr,
                remote_sockaddr_length,
            )
        }
    }

    unsafe fn wsa_connect_ex(
        &self,
        name: *const SOCKADDR,
        name_length: socklen_t,
        send_buffer: *const c_void,
        send_data_length: u32,
        bytes_sent: *mut u32,
        overlapped: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe {
            (**self).wsa_connect_ex(
                name,
                name_length,
                send_buffer,
                send_data_length,
                bytes_sent,
                overlapped,
            )
        }
    }

    unsafe fn wsa_disconnect_ex(
        &self,
        overlapped: *mut OVERLAPPED,
        flags: u32,
        reserved: u32,
    ) -> Poll<io::Result<usize>> {
        unsafe { (**self).wsa_disconnect_ex(overlapped, flags, reserved) }
    }

    unsafe fn wsa_recv_msg(
        &self,
        msg: *mut WSAMSG,
        bytes_received: *mut u32,
        overlapped: *mut OVERLAPPED,
        completion_routine: LPWSAOVERLAPPED_COMPLETION_ROUTINE,
    ) -> Poll<io::Result<usize>> {
        unsafe { (**self).wsa_recv_msg(msg, bytes_received, overlapped, completion_routine) }
    }
}

/// The error of the default [`AsSocketState`] implementations.
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the socket does not provide this winsock extension",
    )
}
