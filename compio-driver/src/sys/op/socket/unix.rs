use std::net::Shutdown;

use super::*;

/// Create a socket.
pub struct CreateSocket {
    pub(crate) domain: i32,
    pub(crate) socket_type: i32,
    pub(crate) protocol: i32,
    pub(crate) opened_fd: Option<Socket2>,
}

impl CreateSocket {
    /// Create [`CreateSocket`].
    pub fn new(domain: i32, socket_type: i32, protocol: i32) -> Self {
        Self {
            domain,
            socket_type,
            protocol,
            opened_fd: None,
        }
    }
}

impl IntoInner for CreateSocket {
    type Inner = Socket2;

    fn into_inner(self) -> Self::Inner {
        self.opened_fd.expect("socket not created")
    }
}

/// Bind a socket to an address.
pub struct Bind<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddr,
}

impl<S> Bind<S> {
    /// Create [`Bind`].
    pub fn new(fd: S, addr: SockAddr) -> Self {
        Self { fd, addr }
    }
}

/// Listen for connections on a socket.
pub struct Listen<S> {
    pub(crate) fd: S,
    pub(crate) backlog: i32,
}

impl<S> Listen<S> {
    /// Create [`Listen`].
    pub fn new(fd: S, backlog: i32) -> Self {
        Self { fd, backlog }
    }
}

/// Shutdown a socket.
pub struct ShutdownSocket<S> {
    pub(crate) fd: S,
    pub(crate) how: Shutdown,
}

impl<S> ShutdownSocket<S> {
    /// Create [`ShutdownSocket`].
    pub fn new(fd: S, how: Shutdown) -> Self {
        Self { fd, how }
    }
}

impl<S: AsFd> ShutdownSocket<S> {
    pub(crate) fn how(&self) -> i32 {
        match self.how {
            Shutdown::Write => libc::SHUT_WR,
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Both => libc::SHUT_RDWR,
        }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        Ok(syscall!(libc::shutdown(self.fd.as_fd().as_raw_fd(), self.how()))? as _)
    }
}

impl CloseSocket {
    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        Ok(syscall!(libc::close(self.fd.as_fd().as_raw_fd()))? as _)
    }
}

/// Accept a connection.
pub struct Accept<S> {
    pub(crate) fd: S,
    pub(crate) buffer: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    pub(crate) accepted_fd: Option<Socket2>,
}

impl<S> Accept<S> {
    /// Create [`Accept`].
    pub fn new(fd: S) -> Self {
        let buffer = SockAddrStorage::zeroed();
        let addr_len = buffer.size_of();
        Self {
            fd,
            buffer,
            addr_len,
            accepted_fd: None,
        }
    }
}

impl<S> IntoInner for Accept<S> {
    type Inner = (Socket2, SockAddr);

    fn into_inner(mut self) -> Self::Inner {
        let socket = self.accepted_fd.take().expect("socket not accepted");
        (socket, unsafe { SockAddr::new(self.buffer, self.addr_len) })
    }
}

#[doc(hidden)]
pub struct RecvVectoredControl {
    pub(crate) msg: libc::msghdr,
    #[allow(dead_code)]
    pub(crate) slices: Vec<SysSlice>,
}

impl Default for RecvVectoredControl {
    fn default() -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            slices: Vec::new(),
        }
    }
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    pub(crate) fn create_control(&mut self, ctrl: &mut RecvVectoredControl) {
        ctrl.slices = self.buffer.sys_slices_mut();
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }
}

#[doc(hidden)]
pub struct SendVectoredControl {
    pub(crate) msg: libc::msghdr,
    #[allow(dead_code)]
    pub(crate) slices: Vec<SysSlice>,
}

impl Default for SendVectoredControl {
    fn default() -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            slices: Vec::new(),
        }
    }
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    pub(crate) fn create_control(&mut self, ctrl: &mut SendVectoredControl) {
        ctrl.slices = self.buffer.sys_slices();
        ctrl.msg.msg_iov = ctrl.slices.as_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }
}

#[doc(hidden)]
pub struct SendMsgControl {
    pub(crate) msg: libc::msghdr,
    #[allow(dead_code)]
    pub(crate) slices: Multi<SysSlice>,
}

impl<S: AsFd> SendToHeader<S> {
    #[allow(dead_code)]
    pub(crate) fn create_control(
        &mut self,
        ctrl: &mut SendMsgControl,
        slices: impl Into<Multi<SysSlice>>,
    ) {
        ctrl.msg.msg_name = self.addr.as_ptr() as _;
        ctrl.msg.msg_namelen = self.addr.len();
        ctrl.slices = slices.into();
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }
}

impl Default for SendMsgControl {
    fn default() -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            slices: Multi::new(),
        }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsg<T, C, S> {
    pub(crate) fn create_control(&mut self, ctrl: &mut SendMsgControl) {
        ctrl.slices = self.buffer.sys_slices().into();
        match self.addr.as_ref() {
            Some(addr) => {
                ctrl.msg.msg_name = addr.as_ptr() as _;
                ctrl.msg.msg_namelen = addr.len();
            }
            None => {
                ctrl.msg.msg_name = std::ptr::null_mut();
                ctrl.msg.msg_namelen = 0;
            }
        }
        ctrl.msg.msg_iov = ctrl.slices.as_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
        ctrl.msg.msg_control = self.control.buf_ptr() as _;
        ctrl.msg.msg_controllen = self.control.buf_len() as _;
    }
}

#[doc(hidden)]
pub struct RecvMsgControl {
    pub(crate) msg: libc::msghdr,
    #[allow(dead_code)]
    pub(crate) slices: Multi<SysSlice>,
}

impl Default for RecvMsgControl {
    fn default() -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            slices: Multi::new(),
        }
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> RecvMsg<T, C, S> {
    pub(crate) fn create_control(&mut self, ctrl: &mut RecvMsgControl) {
        ctrl.slices = Multi::from_vec(self.buffer.sys_slices_mut());
        ctrl.msg.msg_name = &raw mut self.addr as _;
        ctrl.msg.msg_namelen = self.addr.size_of() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
        ctrl.msg.msg_control = self.control.buf_mut_ptr() as _;
        ctrl.msg.msg_controllen = self.control.buf_capacity() as _;
    }

    pub(crate) fn update_control(&mut self, control: &RecvMsgControl) {
        self.name_len = control.msg.msg_namelen as _;
        self.control_len = control.msg.msg_controllen as _;
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvFromVectored<T, S> {
    #[allow(unused)]
    pub(crate) unsafe fn call(&mut self, control: &mut SendMsgControl) -> libc::ssize_t {
        unsafe {
            libc::recvmsg(
                self.header.fd.as_fd().as_raw_fd(),
                &mut control.msg,
                self.header.flags,
            )
        }
    }
}
