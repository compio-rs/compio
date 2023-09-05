use std::{
    io::{IoSlice, IoSliceMut},
    ops::{Deref, DerefMut},
};

use crate::buf::*;

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

impl<T: IoBufMut> WrapBufMut for BufWrapper<T> {
    fn set_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len)
    }
}

impl<T: IoBuf> AsIoSlices for BufWrapper<T> {
    unsafe fn as_io_slices(&self) -> OneOrVec<IoSlice<'static>> {
        OneOrVec::One(IoSlice::new(
            &*(self.buffer.as_slice() as *const _ as *const _),
        ))
    }
}

impl<T: IoBufMut> AsIoSlicesMut for BufWrapper<T> {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut<'static>> {
        OneOrVec::One(IoSliceMut::new(
            &mut *(self.buffer.as_uninit_slice() as *mut _ as *mut _),
        ))
    }
}

impl<T> Deref for BufWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl<T> DerefMut for BufWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
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
    unsafe fn as_io_slices(&self) -> OneOrVec<IoSlice<'static>> {
        OneOrVec::Vec(
            self.buffer
                .iter()
                .map(|buf| IoSlice::new(&*(buf.as_slice() as *const _ as *const _)))
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
    unsafe fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut<'static>> {
        OneOrVec::Vec(
            self.buffer
                .iter_mut()
                .map(|buf| IoSliceMut::new(&mut *(buf.as_uninit_slice() as *mut _ as *mut _)))
                .collect(),
        )
    }
}
