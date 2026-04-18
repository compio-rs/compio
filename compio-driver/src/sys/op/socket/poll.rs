use Interest::*;
use rustix::net::sockopt::socket_error;

use crate::{PollOpCode as OpCode, op::*, sys::driver::*};

unsafe impl OpCode for CreateSocket {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl OpCode for CloseSocket {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Readable, || self.call())
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call())
    }
}

unsafe impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Writable, || self.call())
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        socket_error(&self.fd)??;
        Poll::Ready(Ok(0))
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Readable, || self.call())
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call())
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Writable, || self.call())
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call())
    }
}

impl<S: AsFd> RecvFromHeader<S> {
    pub fn fd(&self) -> RawFd {
        self.fd.as_fd().as_raw_fd()
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = RecvVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.init_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Readable, || self.call(control))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = SendVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.init_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Writable, || self.call(control))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        decide(self.header.fd(), Readable, || self.call())
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call())
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut().into();
        ctrl.msg.msg_name = &raw mut self.header.addr as _;
        ctrl.msg.msg_namelen = self.header.addr.size_of() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        decide(self.header.fd(), Readable, || self.call(control))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        decide(self.header.fd.as_fd().as_raw_fd(), Writable, || self.call())
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call())
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
        decide(self.header.fd.as_fd().as_raw_fd(), Writable, || {
            self.call(control)
        })
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.init_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        decide(self.header.fd(), Readable, || self.call(control))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.header.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
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

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.init_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        decide(self.fd.as_fd().as_raw_fd(), Writable, || self.call(control))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
    }
}
