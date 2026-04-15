use io_uring::{opcode, types::Fd};

use crate::{
    IourOpCode as OpCode, OpEntry,
    sys::{op::*, prelude::*},
};

unsafe impl OpCode for CreateSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Socket::new(
            self.domain,
            self.socket_type | libc::SOCK_CLOEXEC,
            self.protocol,
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        Ok(syscall!(libc::socket(
            self.domain,
            self.socket_type | libc::SOCK_CLOEXEC,
            self.protocol
        ))? as _)
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, res: &io::Result<usize>, _: &Extra) {
        if let Ok(fd) = res {
            // SAFETY: fd is a valid fd returned from kernel
            let fd = unsafe { Socket2::from_raw_fd(*fd as _) };
            self.opened_fd = Some(fd);
        }
    }
}

unsafe impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Bind::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.addr.as_ptr().cast(),
            self.addr.len(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        syscall!(libc::bind(
            self.fd.as_fd().as_raw_fd(),
            self.addr.as_ptr().cast(),
            self.addr.len()
        ))
        .map(|res| res as _)
    }
}

unsafe impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Listen::new(Fd(self.fd.as_fd().as_raw_fd()), self.backlog)
            .build()
            .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        syscall!(libc::listen(self.fd.as_fd().as_raw_fd(), self.backlog)).map(|res| res as _)
    }
}

unsafe impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Shutdown::new(Fd(self.fd.as_fd().as_raw_fd()), self.how())
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl OpCode for CloseSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Close::new(Fd(self.fd.as_fd().as_raw_fd()))
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Accept::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            unsafe { self.buffer.view_as::<libc::sockaddr>() },
            &raw mut self.addr_len,
        )
        .flags(libc::SOCK_CLOEXEC)
        .build()
        .into()
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, res: &io::Result<usize>, _: &Extra) {
        if let Ok(fd) = res {
            // SAFETY: fd is a valid fd returned from kernel
            let fd = unsafe { Socket2::from_raw_fd(*fd as _) };
            self.accepted_fd = Some(fd);
        }
    }
}

unsafe impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Connect::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.addr.as_ptr().cast(),
            self.addr.len(),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Send::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .flags(self.flags)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = SendVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.header.create_control(ctrl, [self.buffer.sys_slice()])
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.header.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.header.flags as _)
            .build()
            .into()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.header.create_control(ctrl, self.buffer.sys_slices())
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.header.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.header.flags as _)
            .build()
            .into()
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        let flags = self.flags;
        let slice = self.buffer.sys_slice_mut();
        opcode::Recv::new(
            Fd(fd),
            slice.ptr() as _,
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .flags(flags)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = RecvVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut control.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

impl<S: AsFd> RecvFromHeader<S> {
    pub fn create_control(
        &mut self,
        ctrl: &mut RecvMsgControl,
        slices: impl Into<Multi<SysSlice>>,
    ) {
        ctrl.msg.msg_name = &raw mut self.addr as _;
        ctrl.msg.msg_namelen = self.addr.size_of() as _;
        ctrl.slices = slices.into();
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr().cast();
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }

    pub fn create_entry(&mut self, control: &mut RecvMsgControl) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut control.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }

    pub fn set_result(&mut self, control: &mut RecvMsgControl) {
        self.name_len = control.msg.msg_namelen;
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.header
            .create_control(ctrl, [self.buffer.sys_slice_mut()])
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.header.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.header.set_result(control);
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.header
            .create_control(ctrl, self.buffer.sys_slices_mut())
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.header.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.header.set_result(control);
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut control.msg)
            .flags(self.flags as _)
            .build()
            .into()
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
