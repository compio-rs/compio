use crate::{
    buf::{BufWrapper, IoBuf, IoBufMut, WrapBuf},
    driver::RawFd,
};

#[derive(Debug)]
pub struct ReadAt<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: BufWrapper<T>,
}

impl<T: IoBufMut> ReadAt<T> {
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer: BufWrapper::new(buffer),
        }
    }

    pub fn into_buffer(self) -> T {
        self.buffer.into_inner()
    }
}

#[derive(Debug)]
pub struct WriteAt<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: BufWrapper<T>,
}

impl<T: IoBuf> WriteAt<T> {
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer: BufWrapper::new(buffer),
        }
    }

    pub fn into_buffer(self) -> T {
        self.buffer.into_inner()
    }
}
