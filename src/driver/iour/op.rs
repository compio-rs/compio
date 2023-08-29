use crate::{
    buf::{AsBuf, AsBufMut, IoBuf, IoBufMut},
    driver::OpCode,
    op::{ReadAt, WriteAt},
};
use io_uring::{
    opcode::{Read, Write},
    squeue::Entry,
    types::Fd,
};

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn create_entry(&mut self) -> Entry {
        let slice = self.buffer.as_buf_mut();
        Read::new(Fd(self.fd), slice.as_mut_ptr(), slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn create_entry(&mut self) -> Entry {
        let slice = self.buffer.as_buf();
        Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}
