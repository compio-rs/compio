use rustix::net::RecvFlags;
use windows_sys::Win32::{
    Networking::WinSock::{
        LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_GETACCEPTEXSOCKADDRS, LPFN_WSARECVMSG,
        SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, SOCKADDR, SOCKADDR_STORAGE,
        SOL_SOCKET, WSAID_ACCEPTEX, WSAID_CONNECTEX, WSAID_GETACCEPTEXSOCKADDRS, WSAID_WSARECVMSG,
        WSAMSG, WSARecv, WSARecvFrom, WSASend, WSASendMsg, WSASendTo, closesocket, setsockopt,
        socklen_t,
    },
    System::IO::OVERLAPPED,
};

use crate::{OpCode, OpType, sys::op::*};

static ACCEPT_EX: OnceLock<LPFN_ACCEPTEX> = OnceLock::new();
static GET_ADDRS: OnceLock<LPFN_GETACCEPTEXSOCKADDRS> = OnceLock::new();

const ACCEPT_ADDR_BUFFER_SIZE: usize = std::mem::size_of::<SOCKADDR_STORAGE>() + 16;
const ACCEPT_BUFFER_SIZE: usize = ACCEPT_ADDR_BUFFER_SIZE * 2;

unsafe impl OpCode for CloseSocket {
    type Control = ();

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(SOCKET, closesocket(self.fd.as_fd().as_raw_fd() as _))? as _,
        ))
    }
}

/// Accept a connection.
pub struct Accept<S, SA> {
    pub(crate) fd: S,
    pub(crate) accept_fd: SA,
    pub(crate) buffer: [u8; ACCEPT_BUFFER_SIZE],
}

impl<S, SA> Accept<S, SA> {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: S, accept_fd: SA) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: [0u8; ACCEPT_BUFFER_SIZE],
        }
    }
}

impl<S: AsFd, SA: AsFd> Accept<S, SA> {
    /// Update accept context.
    pub fn update_context(&self) -> io::Result<()> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(
            SOCKET,
            setsockopt(
                self.accept_fd.as_fd().as_raw_fd() as _,
                SOL_SOCKET,
                SO_UPDATE_ACCEPT_CONTEXT,
                &fd as *const _ as _,
                std::mem::size_of_val(&fd) as _,
            )
        )?;
        Ok(())
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> io::Result<(SA, SockAddr)> {
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

unsafe impl<S: AsFd, SA: AsFd> OpCode for Accept<S, SA> {
    type Control = ();

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
                self.accept_fd.as_fd().as_raw_fd() as _,
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

#[derive(Default)]
#[doc(hidden)]
pub struct RecvControl {
    pub(crate) slice: SysSlice,
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
        let mut flags = self.flags.bits() as _;
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

#[derive(Default)]
#[doc(hidden)]
pub struct RecvVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
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
        let mut flags = self.flags.bits() as _;
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

#[derive(Default)]
#[doc(hidden)]
pub struct SendControl {
    pub(crate) slice: SysSlice,
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
                self.flags.bits() as _,
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

#[derive(Default)]
#[doc(hidden)]
pub struct SendVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
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
                self.flags.bits() as _,
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

#[derive(Default)]
#[doc(hidden)]
pub struct RecvFromControl {
    pub(crate) slice: SysSlice,
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
        let fd = self.header.fd.as_fd().as_raw_fd();
        let mut flags = self.header.flags.bits() as _;
        let mut received = 0;
        let res = unsafe {
            WSARecvFrom(
                fd as _,
                (&raw const control.slice).cast(),
                1,
                &mut received,
                &mut flags,
                &raw mut self.header.addr as *mut SOCKADDR,
                &raw mut self.header.addr_len,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.header.fd.as_fd().as_raw_fd(), optr)
    }
}

#[derive(Default)]
#[doc(hidden)]
pub struct RecvFromVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
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
        let fd = self.header.fd.as_fd().as_raw_fd();
        let mut flags = self.header.flags.bits() as _;
        let mut received = 0;
        let res = unsafe {
            WSARecvFrom(
                fd as _,
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                &mut received,
                &mut flags,
                &raw mut self.header.addr as *mut SOCKADDR,
                &raw mut self.header.addr_len,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.header.fd.as_fd().as_raw_fd(), optr)
    }
}

#[derive(Default)]
#[doc(hidden)]
pub struct SendToControl {
    pub(crate) slice: SysSlice,
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
                self.header.fd.as_fd().as_raw_fd() as _,
                (&raw const control.slice).cast(),
                1,
                &mut sent,
                self.header.flags.bits() as _,
                self.header.addr.as_ptr().cast(),
                self.header.addr.len(),
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.header.fd.as_fd().as_raw_fd(), optr)
    }
}

#[derive(Default)]
#[doc(hidden)]
pub struct SendToVectoredControl {
    pub(crate) slices: Vec<SysSlice>,
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
                self.header.fd.as_fd().as_raw_fd() as _,
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                &mut sent,
                self.header.flags.bits() as _,
                self.header.addr.as_ptr().cast(),
                self.header.addr.len(),
                optr,
                None,
            )
        };
        winsock_result(res, sent)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.header.fd.as_fd().as_raw_fd(), optr)
    }
}

static WSA_RECVMSG: OnceLock<LPFN_WSARECVMSG> = OnceLock::new();

#[derive(Default)]
#[doc(hidden)]
pub struct RecvMsgControl {
    msg: WSAMSG,
    #[allow(dead_code)]
    slices: Vec<SysSlice>,
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut();
        ctrl.msg.dwFlags = self.header.flags.bits() as _;
        ctrl.msg.name = &raw mut self.header.addr as _;
        ctrl.msg.namelen = self.header.addr.size_of() as _;
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
            .get_or_try_init(|| get_wsa_fn(self.header.fd.as_fd().as_raw_fd(), WSAID_WSARECVMSG))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve WSARecvMsg")
            })?;

        let mut received = 0;
        let res = unsafe {
            recvmsg_fn(
                self.header.fd.as_fd().as_raw_fd() as _,
                &raw mut control.msg,
                &mut received,
                optr,
                None,
            )
        };
        winsock_result(res, received)
    }

    fn cancel(&mut self, _: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.header.fd.as_fd().as_raw_fd(), optr)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.header.flags = RecvFlags::from_bits_retain(control.msg.dwFlags);
        self.header.addr_len = control.msg.namelen as socklen_t;
        self.control_len = control.msg.Control.len as _;
    }
}

#[derive(Default)]
#[doc(hidden)]
pub struct SendMsgControl {
    msg: WSAMSG,
    #[allow(dead_code)]
    slices: Vec<SysSlice>,
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
                self.flags.bits() as _,
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
