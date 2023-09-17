use std::io::{IoSlice, IoSliceMut};

use crate::buf::*;

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

impl<'arena, T: IoBuf<'arena>> From<Box<[T]>> for VectoredBufWrapper<'arena, T> {
    fn from(buffers: Box<[T]>) -> Self {
        let io_slices: Box<[IoSlice<'arena>]> = unsafe {
            buffers
                .iter()
                .map(|buf| IoSlice::new(&*(buf.as_slice() as *const _ as *const _)))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        let io_slices_mut: Box<[IoSliceMut<'arena>]> = unsafe {
            buffers
                .iter()
                .map(|buf| IoSliceMut::new(&mut *(buf.as_slice() as *const _ as *mut _)))
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
    unsafe fn as_io_slices(&self) -> OneOrSlice<IoSlice<'arena>> {
        OneOrSlice::Slice(&self.io_slices)
    }
}

impl<'arena, T: IoBufMut<'arena>> AsIoSlicesMut<'arena> for VectoredBufWrapper<'arena, T> {
    unsafe fn as_io_slices_mut(&mut self) -> OneOrSliceMut<IoSliceMut<'arena>> {
        OneOrSliceMut::Slice(&mut self.io_slices_mut)
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
