//! Windows-specific per-socket state.

use std::{
    ffi::c_void,
    io,
    mem::{size_of, size_of_val},
    os::windows::io::AsRawSocket,
    ptr::null_mut,
    task::Poll,
};

use compio_driver::{AsSocketState, win32_result, winsock_result};
use once_cell::sync::OnceCell as OnceLock;
use socket2::socklen_t;
use windows_sys::{
    Win32::{
        Networking::WinSock::{
            LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_DISCONNECTEX, LPFN_GETACCEPTEXSOCKADDRS,
            LPFN_WSARECVMSG, LPWSAOVERLAPPED_COMPLETION_ROUTINE,
            SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKET, SOCKET_ERROR, WSAGetLastError,
            WSAID_ACCEPTEX, WSAID_CONNECTEX, WSAID_DISCONNECTEX, WSAID_GETACCEPTEXSOCKADDRS,
            WSAID_WSARECVMSG, WSAIoctl, WSAMSG,
        },
        System::IO::OVERLAPPED,
    },
    core::GUID,
};

use super::SocketWithState;

/// Strips the [`Option`] from the `LPFN_*` aliases of `windows-sys`: the ioctl
/// writes a bare pointer, a null one meaning "unsupported".
trait FnPtr {
    type Bare;
}

impl<T> FnPtr for Option<T> {
    type Bare = T;
}

type AcceptEx = <LPFN_ACCEPTEX as FnPtr>::Bare;
type GetAcceptExSockaddrs = <LPFN_GETACCEPTEXSOCKADDRS as FnPtr>::Bare;
type ConnectEx = <LPFN_CONNECTEX as FnPtr>::Bare;
type DisconnectEx = <LPFN_DISCONNECTEX as FnPtr>::Bare;
type RecvMsg = <LPFN_WSARECVMSG as FnPtr>::Bare;

/// The winsock extension functions of one socket.
#[derive(Debug, Default)]
struct WsaFns {
    accept_ex: OnceLock<Option<AcceptEx>>,
    get_accept_ex_sockaddrs: OnceLock<Option<GetAcceptExSockaddrs>>,
    connect_ex: OnceLock<Option<ConnectEx>>,
    disconnect_ex: OnceLock<Option<DisconnectEx>>,
    recv_msg: OnceLock<Option<RecvMsg>>,
}

impl WsaFns {
    fn accept_ex(&self, socket: SOCKET) -> io::Result<LPFN_ACCEPTEX> {
        Ok(*self
            .accept_ex
            .get_or_try_init(|| get_wsa_fn::<AcceptEx>(socket, WSAID_ACCEPTEX))?)
    }

    fn get_accept_ex_sockaddrs(&self, socket: SOCKET) -> io::Result<LPFN_GETACCEPTEXSOCKADDRS> {
        Ok(*self.get_accept_ex_sockaddrs.get_or_try_init(|| {
            get_wsa_fn::<GetAcceptExSockaddrs>(socket, WSAID_GETACCEPTEXSOCKADDRS)
        })?)
    }

    fn connect_ex(&self, socket: SOCKET) -> io::Result<LPFN_CONNECTEX> {
        Ok(*self
            .connect_ex
            .get_or_try_init(|| get_wsa_fn::<ConnectEx>(socket, WSAID_CONNECTEX))?)
    }

    fn disconnect_ex(&self, socket: SOCKET) -> io::Result<LPFN_DISCONNECTEX> {
        Ok(*self
            .disconnect_ex
            .get_or_try_init(|| get_wsa_fn::<DisconnectEx>(socket, WSAID_DISCONNECTEX))?)
    }

    fn recv_msg(&self, socket: SOCKET) -> io::Result<LPFN_WSARECVMSG> {
        Ok(*self
            .recv_msg
            .get_or_try_init(|| get_wsa_fn::<RecvMsg>(socket, WSAID_WSARECVMSG))?)
    }
}

/// Query the extension function `guid` of the provider serving `socket`.
///
/// `Ok(None)` means the provider does not implement it.
fn get_wsa_fn<F>(socket: SOCKET, guid: GUID) -> io::Result<Option<F>> {
    let mut fptr = None;
    let mut returned = 0;
    let res = unsafe {
        WSAIoctl(
            socket,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            std::ptr::addr_of!(guid).cast(),
            size_of_val(&guid) as _,
            std::ptr::addr_of_mut!(fptr).cast(),
            size_of::<F>() as _,
            &mut returned,
            null_mut(),
            None,
        )
    };
    if res == SOCKET_ERROR {
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    Ok(fptr)
}

/// Per-socket state, holding the winsock extension cache.
#[derive(Debug, Default)]
pub(super) struct SocketState {
    wsa: Box<WsaFns>,
}

impl SocketState {
    pub(super) fn new() -> Self {
        SocketState::default()
    }
}

impl AsSocketState for SocketWithState {
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
        let socket = self.inner.as_raw_socket() as SOCKET;
        let f = match self.state.wsa.accept_ex(socket) {
            Ok(Some(f)) => f,
            Ok(None) => return Poll::Ready(Err(unsupported())),
            Err(e) => return Poll::Ready(Err(e)),
        };
        let res = unsafe {
            f(
                socket,
                accept_socket,
                output_buffer,
                receive_data_length,
                local_address_length,
                remote_address_length,
                bytes_received,
                overlapped,
            )
        };
        win32_result(res, unsafe { *bytes_received })
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
        let Some(f) = self
            .state
            .wsa
            .get_accept_ex_sockaddrs(self.inner.as_raw_socket() as SOCKET)?
        else {
            return Err(unsupported());
        };
        unsafe {
            f(
                output_buffer.cast_mut(),
                receive_data_length,
                local_address_length,
                remote_address_length,
                local_sockaddr,
                local_sockaddr_length,
                remote_sockaddr,
                remote_sockaddr_length,
            )
        };
        Ok(())
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
        let socket = self.inner.as_raw_socket() as SOCKET;
        let f = match self.state.wsa.connect_ex(socket) {
            Ok(Some(f)) => f,
            Ok(None) => return Poll::Ready(Err(unsupported())),
            Err(e) => return Poll::Ready(Err(e)),
        };
        let res = unsafe {
            f(
                socket,
                name,
                name_length,
                send_buffer,
                send_data_length,
                bytes_sent,
                overlapped,
            )
        };
        win32_result(res, unsafe { *bytes_sent })
    }

    unsafe fn wsa_disconnect_ex(
        &self,
        overlapped: *mut OVERLAPPED,
        flags: u32,
        reserved: u32,
    ) -> Poll<io::Result<usize>> {
        let socket = self.inner.as_raw_socket() as SOCKET;
        let f = match self.state.wsa.disconnect_ex(socket) {
            Ok(Some(f)) => f,
            Ok(None) => return Poll::Ready(Err(unsupported())),
            Err(e) => return Poll::Ready(Err(e)),
        };
        let res = unsafe { f(socket, overlapped, flags, reserved) };
        win32_result(res, 0)
    }

    unsafe fn wsa_recv_msg(
        &self,
        msg: *mut WSAMSG,
        bytes_received: *mut u32,
        overlapped: *mut OVERLAPPED,
        completion_routine: LPWSAOVERLAPPED_COMPLETION_ROUTINE,
    ) -> Poll<io::Result<usize>> {
        let socket = self.inner.as_raw_socket() as SOCKET;
        let f = match self.state.wsa.recv_msg(socket) {
            Ok(Some(f)) => f,
            Ok(None) => return Poll::Ready(Err(unsupported())),
            Err(e) => return Poll::Ready(Err(e)),
        };
        let res = unsafe { f(socket, msg, bytes_received, overlapped, completion_routine) };
        winsock_result(res, unsafe { *bytes_received })
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the socket does not provide this winsock extension",
    )
}
