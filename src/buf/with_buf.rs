use crate::BufResult;
use std::{
    io::{IoSlice, IoSliceMut},
    ops::Deref,
};

/// Trait to get the inner buffer of an operation or a result.
pub trait IntoInner {
    /// The inner type.
    type Inner;

    /// Get the inner buffer.
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

pub trait AsBuf: WrapBuf {
    fn as_buf(&self) -> &[u8];
}

pub trait AsBufMut: WrapBufMut + AsBuf {
    fn as_buf_mut(&mut self) -> &mut [u8];
}

pub trait AsIoSlices: WrapBuf {
    fn as_io_slices(&self) -> OneOrVec<IoSlice>;
}

pub trait AsIoSlicesMut: WrapBufMut + AsIoSlices {
    fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut>;
}

#[derive(Debug)]
pub enum OneOrVec<T> {
    One(T),
    Vec(Vec<T>),
}

impl<T> Deref for OneOrVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Vec(vec) => vec,
        }
    }
}
