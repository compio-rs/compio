use std::pin::Pin;

use io_uring::{
    opcode,
    squeue::Entry,
    types::{Fd, FsyncFlags},
};
use libc::sockaddr_storage;

pub use crate::driver::unix::op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IoBuf, IoBufMut},
    driver::OpCode,
    op::*,
};

impl<'arena, T: IoBufMut<'arena>> OpCode for ReadAt<'arena, T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let fd = Fd(self.fd);
        let slice = self.buffer.as_uninit_slice();
        opcode::Read::new(fd, slice.as_mut_ptr() as _, slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl<'arena, T: IoBuf<'arena>> OpCode for WriteAt<'arena, T> {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl OpCode for Sync {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        opcode::Fsync::new(Fd(self.fd))
            .flags(if self.datasync {
                FsyncFlags::DATASYNC
            } else {
                FsyncFlags::empty()
            })
            .build()
    }
}

impl OpCode for Accept {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        opcode::Accept::new(
            Fd(self.fd),
            &mut self.buffer as *mut sockaddr_storage as *mut libc::sockaddr,
            &mut self.addr_len,
        )
        .build()
    }
}

impl OpCode for Connect {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        opcode::Connect::new(Fd(self.fd), self.addr.as_ptr(), self.addr.len()).build()
    }
}

impl<T: AsIoSlicesMut + Unpin> OpCode for RecvImpl<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        opcode::Readv::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
    }
}

impl<T: AsIoSlices + Unpin> OpCode for SendImpl<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let slices = unsafe { self.buffer.as_io_slices() };
        opcode::Writev::new(Fd(self.fd), slices.as_ptr() as _, slices.len() as _).build()
    }
}

impl<T: AsIoSlicesMut + Unpin> OpCode for RecvFromImpl<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        self.set_msg();
        opcode::RecvMsg::new(Fd(self.fd), &mut self.msg).build()
    }
}

impl<T: AsIoSlices + Unpin> OpCode for SendToImpl<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        self.set_msg();
        opcode::SendMsg::new(Fd(self.fd), &self.msg).build()
    }
}
