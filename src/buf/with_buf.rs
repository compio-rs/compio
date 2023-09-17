use std::{
    io::{IoSlice, IoSliceMut},
    ops::{Deref, DerefMut},
};

use super::{IoBuf, IoBufMut};

pub trait AsIoSlices {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    unsafe fn as_io_slices(&self) -> OneOrSlice<IoSlice<'_>>;
}

pub trait AsIoSlicesMut: AsIoSlices {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    unsafe fn as_io_slices_mut(&mut self) -> OneOrSlice<IoSliceMut<'_>>;

    fn set_init(&mut self, len: usize);
}

#[derive(Debug)]
pub enum OneOrSlice<T> {
    One(T),
    Slice(Box<[T]>),
}

impl<T> Deref for OneOrSlice<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Slice(slice) => slice,
        }
    }
}

impl<T> DerefMut for OneOrSlice<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::One(one) => std::slice::from_mut(one),
            Self::Slice(slice) => slice,
        }
    }
}

impl<'arena, T: IoBuf<'arena>> AsIoSlices for T {
    unsafe fn as_io_slices(&self) -> OneOrSlice<IoSlice<'_>> {
        OneOrSlice::One(IoSlice::new(&*(self.as_slice() as *const _ as *const _)))
    }
}

impl<'arena, T: IoBufMut<'arena>> AsIoSlicesMut for T {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrSlice<IoSliceMut<'_>> {
        OneOrSlice::One(IoSliceMut::new(
            &mut *(self.as_uninit_slice() as *mut _ as *mut _),
        ))
    }

    fn set_init(&mut self, len: usize) {
        self.set_buf_init(len)
    }
}
