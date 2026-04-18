use std::{net::Shutdown, num::NonZeroU32};

use rustix::{
    io::close,
    net::{
        AddressFamily, Protocol, RecvAncillaryBuffer, SendAncillaryBuffer, SocketAddrAny,
        SocketType, acceptfrom_with, bind, connect, listen, recv, recvfrom, recvmsg, send, sendmsg,
        sendmsg_addr, sendto, shutdown, socket_with,
    },
};

use crate::sys::op::*;

impl<S: AsFd> Accept<S> {
    pub(crate) fn call(&mut self) -> io::Result<usize> {
        let (owned, addr) = acceptfrom_with(self.fd.as_fd(), SOCKET_FLAG)?;
        let fd = owned.as_raw_fd();
        let socket: Socket2 = owned.into();

        if cfg!(apple) {
            socket.set_cloexec(true)?;
            socket.set_nonblocking(true)?;
        }

        copy_addr_from(&mut self.buffer, &mut self.addr_len, addr);
        self.accepted_fd = Some(socket);

        Ok(fd as usize)
    }
}

impl<S: AsFd> Connect<S> {
    pub(crate) fn call(&self) -> io::Result<usize> {
        connect(&self.fd, &SockAddrArg(&self.addr))?;
        Ok(0)
    }
}

impl<T: IoBuf, S: AsFd> Send<T, S> {
    pub(crate) fn call(&mut self) -> io::Result<usize> {
        send(self.fd.as_fd(), self.buffer.as_init(), self.flags).map_err(Into::into)
    }
}

impl<T: IoBuf, S: AsFd> SendTo<T, S> {
    pub(crate) fn call(&self) -> io::Result<usize> {
        sendto(
            self.header.fd.as_fd(),
            self.buffer.as_init(),
            self.header.flags,
            &SockAddrArg(&self.header.addr),
        )
        .map_err(Into::into)
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendVectored<T, S> {
    pub(crate) fn call(&self, control: &mut SendVectoredControl) -> io::Result<usize> {
        let mut anc = SendAncillaryBuffer::default();

        sendmsg(
            self.fd.as_fd(),
            io_slice(&control.slices),
            &mut anc,
            self.flags,
        )
        .map_err(Into::into)
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectored<T, S> {
    pub(crate) fn call(&mut self, control: &mut SendMsgControl) -> io::Result<usize> {
        let addr = SockAddrArg(&self.header.addr);
        let mut anc = SendAncillaryBuffer::default();
        let buf = io_slice(&control.slices);

        sendmsg_addr(
            self.header.fd.as_fd(),
            &addr,
            buf,
            &mut anc,
            self.header.flags,
        )
        .map_err(Into::into)
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsg<T, C, S> {
    pub(crate) fn call(&mut self, control: &mut SendMsgControl) -> io::Result<usize> {
        // Both rustix and nix expose api that uses structured AncillaryBuffer
        // building, no way to just throw in an ancillary buf. Fallback to libc here.
        syscall!(libc::sendmsg(
            self.fd.as_fd().as_raw_fd(),
            &control.msg,
            self.flags.bits() as _,
        ))
    }
}

impl<T: IoBufMut, S: AsFd> Recv<T, S> {
    pub(crate) fn call(&mut self) -> io::Result<usize> {
        let (_, len) = recv(self.fd.as_fd(), self.buffer.as_uninit(), self.flags)?;

        Ok(len)
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvVectored<T, S> {
    pub(crate) fn call(&mut self, control: &mut RecvVectoredControl) -> io::Result<usize> {
        let res = recvmsg(
            self.fd.as_fd(),
            io_slice_mut(&mut control.slices),
            &mut RecvAncillaryBuffer::default(),
            self.flags,
        )?;

        // Kernel may truncate and return a larger-than-buffer size
        Ok(res.bytes.min(self.buffer.total_capacity()))
    }
}

impl<S: AsFd> RecvFromHeader<S> {
    pub fn set_addr(&mut self, addr: Option<SocketAddrAny>) {
        copy_addr_from(&mut self.addr, &mut self.addr_len, addr)
    }
}

impl<T: IoBufMut, S: AsFd> RecvFrom<T, S> {
    pub(crate) fn call(&mut self) -> io::Result<usize> {
        let (_, len, addr) = recvfrom(&self.header.fd, self.buffer.as_uninit(), self.header.flags)?;

        self.header.set_addr(addr);

        Ok(len.min(self.buffer.buf_capacity()))
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> RecvMsg<T, C, S> {
    pub(crate) fn call(&mut self, control: &mut RecvMsgControl) -> io::Result<usize> {
        let res = syscall!(libc::recvmsg(
            self.header.fd.as_fd().as_raw_fd(),
            &raw mut control.msg,
            self.header.flags.bits() as _,
        ))?;

        self.update_control(control);

        Ok(res)
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvFromVectored<T, S> {
    pub(crate) fn call(&mut self, control: &mut RecvMsgControl) -> io::Result<usize> {
        let res = recvmsg(
            &self.header.fd,
            io_slice_mut(&mut control.slices),
            &mut RecvAncillaryBuffer::default(),
            self.header.flags,
        )?;

        self.header.set_addr(res.address);

        Ok(res.bytes)
    }
}

/// Create a socket.
pub struct CreateSocket {
    pub(crate) domain: AddressFamily,
    pub(crate) socket_type: SocketType,
    pub(crate) protocol: Option<Protocol>,
    pub(crate) opened_fd: Option<Socket2>,
}

impl CreateSocket {
    /// Create [`CreateSocket`].
    pub fn new(domain: i32, socket_type: i32, protocol: i32) -> Self {
        let domain = AddressFamily::from_raw(domain as _);
        let socket_type = SocketType::from_raw(socket_type as _);
        let protocol = NonZeroU32::new(protocol as _).map(Protocol::from_raw);

        Self {
            domain,
            socket_type,
            protocol,
            opened_fd: None,
        }
    }

    pub(crate) fn call(&mut self) -> io::Result<usize> {
        let owned = socket_with(self.domain, self.socket_type, SOCKET_FLAG, self.protocol)?;
        let fd = owned.as_raw_fd();
        let socket: Socket2 = owned.into();

        #[cfg(apple)]
        {
            socket.set_cloexec(true)?;
            socket.set_nosigpipe(true)?;
            socket.set_nonblocking(true)?;
        }

        self.opened_fd = Some(socket);
        Ok(fd as _)
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

impl<S: AsFd> Bind<S> {
    pub(crate) fn call(&self) -> io::Result<usize> {
        bind(self.fd.as_fd(), &SockAddrArg(&self.addr))?;
        Ok(0)
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

impl<S: AsFd> Listen<S> {
    pub(crate) fn call(&self) -> io::Result<usize> {
        listen(self.fd.as_fd(), self.backlog)?;
        Ok(0)
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
    #[cfg(io_uring)]
    pub(crate) fn how(&self) -> i32 {
        match self.how {
            Shutdown::Write => libc::SHUT_WR,
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Both => libc::SHUT_RDWR,
        }
    }

    pub(crate) fn call(&mut self) -> io::Result<usize> {
        let how = match self.how {
            Shutdown::Write => rustix::net::Shutdown::Write,
            Shutdown::Read => rustix::net::Shutdown::Read,
            Shutdown::Both => rustix::net::Shutdown::Both,
        };
        shutdown(&self.fd, how)?;
        Ok(0)
    }
}

impl CloseSocket {
    pub(crate) fn call(&mut self) -> io::Result<usize> {
        unsafe { close(self.fd.as_raw_fd()) };
        Ok(0)
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
    pub(crate) fn init_control(&mut self, ctrl: &mut RecvVectoredControl) {
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
    pub(crate) fn init_control(&mut self, ctrl: &mut SendVectoredControl) {
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
    pub(crate) fn init_control(&mut self, ctrl: &mut SendMsgControl) {
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
    pub(crate) fn init_control(&mut self, ctrl: &mut RecvMsgControl) {
        ctrl.slices = Multi::from_vec(self.buffer.sys_slices_mut());
        ctrl.msg.msg_name = &raw mut self.header.addr as _;
        ctrl.msg.msg_namelen = self.header.addr.size_of() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
        ctrl.msg.msg_control = self.control.buf_mut_ptr() as _;
        ctrl.msg.msg_controllen = self.control.buf_capacity() as _;
    }

    pub(crate) fn update_control(&mut self, control: &RecvMsgControl) {
        self.header.addr_len = control.msg.msg_namelen as _;
        self.control_len = control.msg.msg_controllen as _;
    }
}
