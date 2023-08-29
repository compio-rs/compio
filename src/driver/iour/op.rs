use crate::{
    buf::{IoBuf, IoBufMut, WithBuf, WithBufMut},
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
        self.buffer.with_buf_mut(|ptr, len| {
            Read::new(Fd(self.fd), ptr, len as _)
                .offset(self.offset as _)
                .build()
        })
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn create_entry(&mut self) -> io_uring::squeue::Entry {
        self.buffer.with_buf(|ptr, len| {
            Write::new(Fd(self.fd), ptr, len as _)
                .offset(self.offset as _)
                .build()
        })
    }
}
