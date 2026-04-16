use crate::{
    PollOpCode as OpCode,
    op::*,
    sys::{driver::*, prelude::*},
    syscall,
};

impl CreateSocket {
    unsafe fn call(&mut self, _: &mut ()) -> io::Result<libc::c_int> {
        #[allow(unused_mut)]
        let mut ty: i32 = self.socket_type;
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "cygwin",
        ))]
        {
            ty |= libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK;
        }
        let fd = syscall!(libc::socket(self.domain, ty, self.protocol))?;
        let socket = unsafe { Socket2::from_raw_fd(fd) };
        #[cfg(not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "espidf",
            target_os = "vita",
            target_os = "cygwin",
        )))]
        socket.set_cloexec(true)?;
        #[cfg(target_vendor = "apple")]
        socket.set_nosigpipe(true)?;
        #[cfg(not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "cygwin",
        )))]
        socket.set_nonblocking(true)?;
        self.opened_fd = Some(socket);
        Ok(fd)
    }
}

unsafe impl OpCode for CreateSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(unsafe { self.call(control)? } as _))
    }
}

unsafe impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(
            syscall!(libc::bind(
                self.fd.as_fd().as_raw_fd(),
                self.addr.as_ptr().cast(),
                self.addr.len() as socklen_t
            ))
            .map(|res| res as _),
        )
    }
}

unsafe impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(
            syscall!(libc::listen(self.fd.as_fd().as_raw_fd(), self.backlog)).map(|res| res as _),
        )
    }
}

unsafe impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl OpCode for CloseSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

impl<S: AsFd> Accept<S> {
    // If the first call succeeds, there won't be another call.
    unsafe fn call(&mut self, _: &mut ()) -> libc::c_int {
        || -> io::Result<libc::c_int> {
            #[cfg(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "linux",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "cygwin",
            ))]
            {
                let fd = syscall!(libc::accept4(
                    self.fd.as_fd().as_raw_fd(),
                    &raw mut self.buffer as *mut _,
                    &raw mut self.addr_len,
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                self.accepted_fd = Some(socket);
                Ok(fd)
            }
            #[cfg(not(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "linux",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "cygwin",
            )))]
            {
                let fd = syscall!(libc::accept(
                    self.fd.as_fd().as_raw_fd(),
                    &raw mut self.buffer as *mut _,
                    &raw mut self.addr_len,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                socket.set_cloexec(true)?;
                socket.set_nonblocking(true)?;
                self.accepted_fd = Some(socket);
                Ok(fd)
            }
        }()
        .unwrap_or(-1)
    }
}

unsafe impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        syscall!(
            libc::connect(
                self.fd.as_fd().as_raw_fd(),
                self.addr.as_ptr().cast(),
                self.addr.len()
            ),
            wait_writable(self.fd.as_fd().as_raw_fd())
        )
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(libc::getsockopt(
            self.fd.as_fd().as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut err as *mut _ as *mut _,
            &mut err_len
        ))?;

        let res = if err == 0 {
            Ok(0)
        } else {
            Err(io::Error::from_raw_os_error(err))
        };
        Poll::Ready(res)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let flags = self.flags;
        let slice = self.buffer.sys_slice_mut();
        syscall!(break libc::recv(fd, slice.ptr() as _, slice.len(), flags))
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        syscall!(
            break libc::send(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len(),
                self.flags,
            )
        )
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvVectored<T, S> {
    unsafe fn call(&mut self, control: &mut RecvVectoredControl) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = RecvVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendVectored<T, S> {
    unsafe fn call(&self, control: &mut SendVectoredControl) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = SendVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_writable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

impl<T: IoBufMut, S: AsFd> RecvFrom<T, S> {
    unsafe fn call(&mut self, _: &mut ()) -> libc::ssize_t {
        let fd = self.header.fd.as_fd().as_raw_fd();
        let slice = self.buffer.sys_slice_mut();

        unsafe {
            libc::recvfrom(
                fd,
                slice.ptr() as _,
                slice.len(),
                self.header.flags,
                &raw mut self.header.addr as _,
                &raw mut self.header.name_len,
            )
        }
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.header.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut().into();
        ctrl.msg.msg_name = &raw mut self.header.addr as _;
        ctrl.msg.msg_namelen = self.header.addr.size_of() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.header.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.header.name_len = control.msg.msg_namelen;
    }
}

impl<T: IoBuf, S: AsFd> SendTo<T, S> {
    unsafe fn call(&self) -> libc::ssize_t {
        let slice = self.buffer.as_init();
        unsafe {
            libc::sendto(
                self.header.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len(),
                self.header.flags,
                self.header.addr.as_ptr().cast(),
                self.header.addr.len(),
            )
        }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        syscall!(
            self.call(),
            wait_writable(self.header.fd.as_fd().as_raw_fd())
        )
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call())
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectored<T, S> {
    unsafe fn call(&mut self, control: &mut SendMsgControl) -> libc::ssize_t {
        unsafe {
            libc::sendmsg(
                self.header.fd.as_fd().as_raw_fd(),
                &control.msg,
                self.header.flags,
            )
        }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices().into();
        ctrl.msg.msg_name = self.header.addr.as_ptr() as _;
        ctrl.msg.msg_namelen = self.header.addr.len() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.header.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_writable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> RecvMsg<T, C, S> {
    unsafe fn call(&mut self, control: &mut RecvMsgControl) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.update_control(control);
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsg<T, C, S> {
    unsafe fn call(&mut self, control: &mut SendMsgControl) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_writable(fd))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}
