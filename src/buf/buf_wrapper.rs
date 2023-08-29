use crate::buf::*;
use std::io::{IoSlice, IoSliceMut};

#[derive(Debug)]
pub struct BufWrapper<T> {
    buffer: T,
}

impl<T> IntoInner for BufWrapper<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> WrapBuf for BufWrapper<T> {
    fn new(buffer: T) -> Self {
        Self { buffer }
    }
}

impl<T: IoBuf> AsBuf for BufWrapper<T> {
    fn as_buf(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.buffer.as_buf_ptr(), self.buffer.buf_len()) }
    }
}

impl<T: IoBufMut> WrapBufMut for BufWrapper<T> {
    fn set_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len)
    }
}

impl<T: IoBufMut> AsBufMut for BufWrapper<T> {
    fn as_buf_mut(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(
                self.buffer.as_buf_mut_ptr().add(self.buffer.buf_len()),
                self.buffer.buf_capacity() - self.buffer.buf_len(),
            )
        }
    }
}

impl<T: IoBuf> AsIoSlices for BufWrapper<T> {
    fn as_io_slices(&self) -> OneOrVec<IoSlice> {
        OneOrVec::One(IoSlice::new(self.as_buf()))
    }
}

impl<T: IoBufMut> AsIoSlicesMut for BufWrapper<T> {
    fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut> {
        OneOrVec::One(IoSliceMut::new(self.as_buf_mut()))
    }
}

#[derive(Debug)]
pub struct VectoredBufWrapper<T> {
    buffer: Vec<T>,
}

impl<T> IntoInner for VectoredBufWrapper<T> {
    type Inner = Vec<T>;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> WrapBuf for VectoredBufWrapper<T> {
    fn new(buffer: Vec<T>) -> Self {
        Self { buffer }
    }
}

impl<T: IoBuf> AsIoSlices for VectoredBufWrapper<T> {
    fn as_io_slices(&self) -> OneOrVec<IoSlice> {
        OneOrVec::Vec(
            self.buffer
                .iter()
                .map(|buf| {
                    IoSlice::new(unsafe {
                        std::slice::from_raw_parts(buf.as_buf_ptr(), buf.buf_len())
                    })
                })
                .collect(),
        )
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

impl<T: IoBufMut> AsIoSlicesMut for VectoredBufWrapper<T> {
    fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut> {
        OneOrVec::Vec(
            self.buffer
                .iter_mut()
                .map(|buf| {
                    IoSliceMut::new(unsafe {
                        std::slice::from_raw_parts_mut(
                            buf.as_buf_mut_ptr().add(buf.buf_len()),
                            buf.buf_capacity() - buf.buf_len(),
                        )
                    })
                })
                .collect(),
        )
    }
}
