use crate::buf::*;
use std::io::{IoSlice, IoSliceMut};

#[derive(Debug)]
pub struct BufWrapper<T> {
    buffer: T,
}

impl<T: IoBuf> WrapBuf for BufWrapper<T> {
    type Buffer = T;

    fn new(buffer: Self::Buffer) -> Self {
        Self { buffer }
    }

    fn into_inner(self) -> Self::Buffer {
        self.buffer
    }
}

impl<T: IoBuf> WithBuf for BufWrapper<T> {
    fn with_buf<R>(&self, f: impl FnOnce(*const u8, usize) -> R) -> R {
        f(self.buffer.as_buf_ptr(), self.buffer.buf_len())
    }
}

impl<T: IoBufMut> WrapBufMut for BufWrapper<T> {
    fn set_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len)
    }
}

impl<T: IoBufMut> WithBufMut for BufWrapper<T> {
    fn with_buf_mut<R>(&mut self, f: impl FnOnce(*mut u8, usize) -> R) -> R {
        f(
            unsafe { self.buffer.as_buf_mut_ptr().add(self.buffer.buf_len()) },
            self.buffer.buf_capacity() - self.buffer.buf_len(),
        )
    }
}

impl<T: IoBuf> WithWsaBuf for BufWrapper<T> {
    fn with_wsa_buf<R>(&self, f: impl FnOnce(*const IoSlice, usize) -> R) -> R {
        let buffer = IoSlice::new(unsafe {
            std::slice::from_raw_parts(self.buffer.as_buf_ptr(), self.buffer.buf_len())
        });
        f(&buffer, 1)
    }
}

impl<T: IoBufMut> WithWsaBufMut for BufWrapper<T> {
    fn with_wsa_buf_mut<R>(&mut self, f: impl FnOnce(*const IoSliceMut, usize) -> R) -> R {
        let buffer = IoSliceMut::new(unsafe {
            std::slice::from_raw_parts_mut(
                self.buffer.as_buf_mut_ptr().add(self.buffer.buf_len()),
                self.buffer.buf_capacity() - self.buffer.buf_len(),
            )
        });
        f(&buffer, 1)
    }
}

#[derive(Debug)]
pub struct VectoredBufWrapper<T> {
    buffer: Vec<T>,
}

impl<T: IoBuf> WrapBuf for VectoredBufWrapper<T> {
    type Buffer = Vec<T>;

    fn new(buffer: Self::Buffer) -> Self {
        Self { buffer }
    }

    fn into_inner(self) -> Self::Buffer {
        self.buffer
    }
}

impl<T: IoBuf> WithWsaBuf for VectoredBufWrapper<T> {
    fn with_wsa_buf<R>(&self, f: impl FnOnce(*const IoSlice, usize) -> R) -> R {
        let buffers = self
            .buffer
            .iter()
            .map(|buf| {
                IoSlice::new(unsafe { std::slice::from_raw_parts(buf.as_buf_ptr(), buf.buf_len()) })
            })
            .collect::<Vec<_>>();
        f(buffers.as_ptr(), buffers.len())
    }
}

impl<T: IoBufMut> WrapBufMut for VectoredBufWrapper<T> {
    fn set_init(&mut self, mut len: usize) {
        for buf in self.buffer.iter_mut() {
            let capacity = buf.buf_capacity();
            if len >= capacity {
                buf.set_buf_init(capacity);
                len -= capacity;
            } else {
                buf.set_buf_init(len);
                len = 0;
            }
        }
    }
}

impl<T: IoBufMut> WithWsaBufMut for VectoredBufWrapper<T> {
    fn with_wsa_buf_mut<R>(&mut self, f: impl FnOnce(*const IoSliceMut, usize) -> R) -> R {
        let buffers = self
            .buffer
            .iter_mut()
            .map(|buf| {
                IoSliceMut::new(unsafe {
                    std::slice::from_raw_parts_mut(
                        buf.as_buf_mut_ptr().add(buf.buf_len()),
                        buf.buf_capacity() - buf.buf_len(),
                    )
                })
            })
            .collect::<Vec<_>>();
        f(buffers.as_ptr(), buffers.len())
    }
}
