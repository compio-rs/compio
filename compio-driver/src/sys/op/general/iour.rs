use io_uring::{opcode, types::Fd};

use crate::{IourOpCode as OpCode, OpEntry, sys::op::*};

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = VectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Readv::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Write::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = VectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Writev::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.buffer.sys_slice_mut();
        opcode::Read::new(
            Fd(fd),
            slice.ptr() as _,
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = Fd(self.fd.as_fd().as_raw_fd());
        let slice = self.buffer.sys_slice_mut();
        opcode::Read::new(
            fd,
            slice.ptr() as _,
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = VectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Readv::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Write::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = VectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices();
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Writev::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let flags = match self.interest {
            Interest::Readable => libc::POLLIN,
            Interest::Writable => libc::POLLOUT,
        };
        opcode::PollAdd::new(Fd(self.fd.as_fd().as_raw_fd()), flags as _)
            .build()
            .into()
    }
}

unsafe impl OpCode for Pipe {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Pipe::new(self.fds.as_mut_ptr().cast())
            .flags(libc::O_CLOEXEC as _)
            .build()
            .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        self.call()
    }
}
