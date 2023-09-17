use std::{
    io::{IoSlice, IoSliceMut},
    ops::{Deref, DerefMut},
};

use super::{IoBuf, IoBufMut};

pub trait AsIoSlices<'arena>: 'arena {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    unsafe fn as_io_slices(&self) -> OneOrSlice<IoSlice<'arena>>;
}

pub trait AsIoSlicesMut<'arena>: AsIoSlices<'arena> {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    unsafe fn as_io_slices_mut(&mut self) -> OneOrSliceMut<IoSliceMut<'arena>>;

    fn set_init(&mut self, len: usize);
}

#[derive(Debug)]
pub enum OneOrSlice<'arena, T> {
    One(T),
    Slice(&'arena [T]),
}

impl<'arena, T> Deref for OneOrSlice<'arena, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Slice(slice) => *slice,
        }
    }
}

#[derive(Debug)]
pub enum OneOrSliceMut<'arena, T> {
    One(T),
    Slice(&'arena mut [T]),
}

impl<'arena, T> Deref for OneOrSliceMut<'arena, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Slice(slice) => *slice,
        }
    }
}

impl<'arena, T> DerefMut for OneOrSliceMut<'arena, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::One(one) => std::slice::from_mut(one),
            Self::Slice(slice) => *slice,
        }
    }
}

impl<'arena, T: IoBuf<'arena>> AsIoSlices<'arena> for T {
    unsafe fn as_io_slices(&self) -> OneOrSlice<IoSlice<'arena>> {
        OneOrSlice::One(IoSlice::new(&*(self.as_slice() as *const _ as *const _)))
    }
}

impl<'arena, T: IoBufMut<'arena>> AsIoSlicesMut<'arena> for T {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrSliceMut<IoSliceMut<'arena>> {
        OneOrSliceMut::One(IoSliceMut::new(
            &mut *(self.as_uninit_slice() as *mut _ as *mut _),
        ))
    }

    fn set_init(&mut self, len: usize) {
        self.set_buf_init(len)
    }
}
