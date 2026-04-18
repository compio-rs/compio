use rustix::io::{pread, preadv, pwrite, pwritev, read, readv, write, writev};

use crate::{Decision, OpType, PollOpCode as OpCode, sys::op::*};

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = AioControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.init_mut(&self.fd, &mut self.buffer, self.offset);
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        ctrl.decide_read()
    }

    fn op_type(&mut self, ctrl: &mut Self::Control) -> Option<crate::OpType> {
        ctrl.op_type()
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| {
            pread(self.fd.as_fd(), self.buffer.as_uninit(), self.offset).map(|(init, _)| init.len())
        })
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = AioControl<VectoredControl>;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.init_vec_mut(&self.fd, &mut self.buffer, self.offset);
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        ctrl.decide_read()
    }

    fn op_type(&mut self, ctrl: &mut Self::Control) -> Option<crate::OpType> {
        ctrl.op_type()
    }

    fn operate(&mut self, ctrl: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| preadv(self.fd.as_fd(), ctrl.io_slices_mut(), self.offset))
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = AioControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.init(&self.fd, &self.buffer, self.offset);
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        ctrl.decide_write()
    }

    fn op_type(&mut self, ctrl: &mut Self::Control) -> Option<crate::OpType> {
        ctrl.op_type()
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| pwrite(self.fd.as_fd(), self.buffer.as_init(), self.offset))
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = AioControl<VectoredControl>;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.init_vec(&self.fd, &self.buffer, self.offset);
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        ctrl.decide_write()
    }

    fn op_type(&mut self, ctrl: &mut Self::Control) -> Option<crate::OpType> {
        ctrl.op_type()
    }

    fn operate(&mut self, ctrl: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| pwritev(self.fd.as_fd(), ctrl.io_slices(), self.offset))
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| read(self.fd.as_fd(), self.buffer.as_uninit()).map(|(init, _)| init.len()))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = VectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| readv(self.fd.as_fd(), control.io_slices_mut()))
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| write(self.fd.as_fd(), self.buffer.as_init()))
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = VectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| writev(self.fd.as_fd(), control.io_slices()))
    }
}

unsafe impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_for(
            self.fd.as_fd().as_raw_fd(),
            self.interest,
        ))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }
}

unsafe impl OpCode for Pipe {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}
