use std::io::{IoSlice, IoSliceMut};

use crate::BufResult;

pub trait IntoInner {
    type Inner;

    fn into_inner(self) -> Self::Inner;
}

impl<T: IntoInner, O> IntoInner for BufResult<O, T> {
    type Inner = BufResult<O, T::Inner>;

    fn into_inner(self) -> Self::Inner {
        (self.0, self.1.into_inner())
    }
}

pub trait WrapBuf: IntoInner {
    fn new(buffer: Self::Inner) -> Self;
}

pub trait WrapBufMut {
    fn set_init(&mut self, len: usize);
}

pub trait WithBuf: WrapBuf {
    fn with_buf<R>(&self, f: impl FnOnce(*const u8, usize) -> R) -> R;
}

pub trait WithBufMut: WrapBufMut + WithBuf {
    fn with_buf_mut<R>(&mut self, f: impl FnOnce(*mut u8, usize) -> R) -> R;
}

pub trait WithWsaBuf: WrapBuf {
    fn with_wsa_buf<R>(&self, f: impl FnOnce(*const IoSlice, usize) -> R) -> R;
}

pub trait WithWsaBufMut: WrapBufMut + WithWsaBuf {
    fn with_wsa_buf_mut<R>(&mut self, f: impl FnOnce(*const IoSliceMut, usize) -> R) -> R;
}
