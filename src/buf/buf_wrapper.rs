use std::{
    io::{IoSlice, IoSliceMut},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::buf::*;

#[derive(Debug)]
pub struct BufWrapper<'arena, T: 'arena> {
    buffer: T,
    _lifetime: PhantomData<&'arena ()>,
}

// The buffer won't be extended.
impl<'arena, T: IoBuf<'arena>> Unpin for BufWrapper<'arena, T> {}

impl<'arena, T> IntoInner for BufWrapper<'arena, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<'arena, T: IoBuf<'arena>> WrapBuf for BufWrapper<'arena, T> {
    fn new(buffer: T) -> Self {
        Self {
            buffer,
            _lifetime: PhantomData,
        }
    }
}

impl<'arena, T: IoBufMut<'arena>> WrapBufMut for BufWrapper<'arena, T> {
    fn set_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len)
    }
}

impl<'arena, T: IoBuf<'arena>> AsIoSlices for BufWrapper<'arena, T> {
    unsafe fn as_io_slices(&self) -> OneOrVec<IoSlice<'_>> {
        OneOrVec::One(IoSlice::new(
            &*(self.buffer.as_slice() as *const _ as *const _),
        ))
    }
}

impl<'arena, T: IoBufMut<'arena>> AsIoSlicesMut for BufWrapper<'arena, T> {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut<'_>> {
        OneOrVec::One(IoSliceMut::new(
            &mut *(self.buffer.as_uninit_slice() as *mut _ as *mut _),
        ))
    }
}

impl<'arena, T> Deref for BufWrapper<'arena, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl<'arena, T> DerefMut for BufWrapper<'arena, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}

#[derive(Debug)]
pub struct VectoredBufWrapper<T> {
    buffer: Vec<T>,
}

// The buffer won't be extended.
impl<T: IoBuf<'static>> Unpin for VectoredBufWrapper<T> {}

impl<T> IntoInner for VectoredBufWrapper<T> {
    // we require vec from global allocator and 'static lifetime until allocator_api
    // becomes stable
    type Inner = Vec<T>;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf<'static>> WrapBuf for VectoredBufWrapper<T> {
    fn new(buffer: Vec<T>) -> Self {
        Self { buffer }
    }
}

impl<T: IoBuf<'static>> AsIoSlices for VectoredBufWrapper<T> {
    unsafe fn as_io_slices(&self) -> OneOrVec<IoSlice<'static>> {
        OneOrVec::Vec(
            self.buffer
                .iter()
                .map(|buf| IoSlice::new(&*(buf.as_slice() as *const _ as *const _)))
                .collect(),
        )
    }
}

impl<T: IoBufMut<'static>> WrapBufMut for VectoredBufWrapper<T> {
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

impl<T: IoBufMut<'static>> AsIoSlicesMut for VectoredBufWrapper<T> {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrVec<IoSliceMut<'static>> {
        OneOrVec::Vec(
            self.buffer
                .iter_mut()
                .map(|buf| IoSliceMut::new(&mut *(buf.as_uninit_slice() as *mut _ as *mut _)))
                .collect(),
        )
    }
}
