use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
};

use crate::buf::*;

/// Holds an immutable IO buffer and IoSlice.
#[derive(Debug)]
pub struct BufWrapper<'arena, T: 'arena> {
    buffer: T,
    io_slice: [IoSlice<'arena>; 1],
}

impl<'arena, T: IoBuf<'arena>> From<T> for BufWrapper<'arena, T> {
    fn from(buffer: T) -> Self {
        // SAFETY: buffer Unpin and could be self referenced
        let io_slice = [IoSlice::new(unsafe {
            &*(buffer.as_slice() as *const [u8])
        })];
        Self { buffer, io_slice }
    }
}

impl<T> IntoInner for BufWrapper<'_, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Holds an mutable IO buffer and IoSliceMut.
#[derive(Debug)]
pub struct BufWrapperMut<'arena, T: 'arena> {
    buffer: T,
    io_slice_mut: [IoSliceMut<'arena>; 1],
}

impl<T> IntoInner for BufWrapperMut<'_, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<'arena, T: IoBufMut<'arena>> From<T> for BufWrapperMut<'arena, T> {
    fn from(mut buffer: T) -> Self {
        // SAFETY: buffer Unpin and could be self referenced
        let io_slice_mut = [IoSliceMut::new(unsafe {
            slice_assume_init_mut::<'arena>(buffer.as_uninit_slice() as *mut [MaybeUninit<u8>])
        })];
        Self {
            buffer,
            io_slice_mut,
        }
    }
}

unsafe fn slice_assume_init_mut<'arena>(slice: *mut [MaybeUninit<u8>]) -> &'arena mut [u8] {
    unsafe { &mut *(slice as *mut [u8]) }
}

impl<'arena, T: IoBuf<'arena>> AsIoSlices<'arena> for BufWrapper<'arena, T> {
    unsafe fn as_io_slices(&self) -> &[IoSlice<'arena>] {
        &self.io_slice
    }
}

impl<'arena, T: IoBufMut<'arena>> AsIoSlicesMut<'arena> for BufWrapperMut<'arena, T> {
    unsafe fn as_io_slices_mut(&mut self) -> &mut [IoSliceMut<'arena>] {
        &mut self.io_slice_mut
    }

    fn set_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len)
    }
}

/// Fixed slice of IO buffers.
#[derive(Debug)]
pub struct VectoredBufWrapper<'arena, T: 'arena> {
    buffers: Box<[T]>,
    io_slices: Box<[IoSlice<'arena>]>,
    io_slices_mut: Box<[IoSliceMut<'arena>]>,
}

impl<T> IntoInner for VectoredBufWrapper<'_, T> {
    type Inner = Box<[T]>;

    fn into_inner(self) -> Self::Inner {
        self.buffers
    }
}

impl<'arena, T: IoBufMut<'arena>> From<Box<[T]>> for VectoredBufWrapper<'arena, T> {
    fn from(mut buffers: Box<[T]>) -> Self {
        let io_slices: Box<[IoSlice<'arena>]> = unsafe {
            buffers
                .iter()
                .map(|buf| IoSlice::new(&*(buf.as_slice() as *const _ as *const _)))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        let io_slices_mut: Box<[IoSliceMut<'arena>]> = unsafe {
            buffers
                .iter_mut()
                .map(|buf| IoSliceMut::new(&mut *(buf.as_uninit_slice() as *mut _ as *mut _)))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        Self {
            buffers,
            io_slices,
            io_slices_mut,
        }
    }
}

impl<'arena, T: IoBuf<'arena>> AsIoSlices<'arena> for VectoredBufWrapper<'arena, T> {
    unsafe fn as_io_slices(&self) -> &[IoSlice<'_>] {
        &self.io_slices
    }
}

impl<'arena, T: IoBufMut<'arena>> AsIoSlicesMut<'arena> for VectoredBufWrapper<'arena, T> {
    unsafe fn as_io_slices_mut(&mut self) -> &mut [IoSliceMut<'arena>] {
        &mut self.io_slices_mut
    }

    fn set_init(&mut self, mut len: usize) {
        for buf in self.buffers.iter_mut() {
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
