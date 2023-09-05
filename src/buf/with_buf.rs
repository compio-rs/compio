use std::{
    io::{IoSlice, IoSliceMut},
    ops::{Deref, DerefMut},
};

use crate::buf::IntoInner;

pub trait WrapBuf: IntoInner {
    fn new(buffer: Self::Inner) -> Self;
}

pub trait WrapBufMut {
    fn set_init(&mut self, len: usize);
}

pub trait AsIoSlices: WrapBuf {
    unsafe fn as_io_slices(&self) -> OneOrVec<IoSlice<'static>>;
}

pub trait AsIoSlicesMut: WrapBufMut + AsIoSlices {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut<'static>>;
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

impl<T> DerefMut for OneOrVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::One(one) => std::slice::from_mut(one),
            Self::Vec(vec) => vec,
        }
    }
}
