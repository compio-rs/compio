use std::mem::MaybeUninit;

use crate::{IntoInner, IoBuf, IoBufMut, IoSlice, IoSliceMut, SetBufInit, t_alloc};

/// A trait for vectored buffers.
pub trait IoVectoredBuf: 'static {
    /// Collected [`IoSlice`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn io_slices(&self) -> Vec<IoSlice> {
        self.iter_io_slice().collect()
    }

    /// An iterator over the [`IoSlice`]s.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice>;

    /// An iterator over slices.
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        unsafe {
            self.iter_io_slice()
                .map(|slice| std::slice::from_raw_parts(slice.as_ptr(), slice.len()))
        }
    }

    /// Wrap self into an owned iterator.
    fn owned_iter(self) -> Result<VectoredBufIter<Self>, Self>
    where
        Self: Sized,
    {
        let len = unsafe { self.iter_io_slice().count() };
        if len > 0 {
            Ok(VectoredBufIter {
                buf: self,
                index: 0,
                len,
                filled: 0,
                cur_filled: 0,
            })
        } else {
            Err(self)
        }
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static [T] {
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter().map(|buf| buf.as_io_slice())
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static mut [T] {
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter().map(|buf| buf.as_io_slice())
    }
}

impl<T: IoBuf, const N: usize> IoVectoredBuf for [T; N] {
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter().map(|buf| buf.as_io_slice())
    }
}

impl<T: IoBuf, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static> IoVectoredBuf
    for t_alloc!(Vec, T, A)
{
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter().map(|buf| buf.as_io_slice())
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for arrayvec::ArrayVec<T, N> {
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter().map(|buf| buf.as_io_slice())
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter().map(|buf| buf.as_io_slice())
    }
}

impl<T: IoBuf, Rest: IoVectoredBuf> IoVectoredBuf for (T, Rest) {
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        std::iter::once(self.0.as_io_slice()).chain(self.1.iter_io_slice())
    }
}

impl IoVectoredBuf for () {
    unsafe fn iter_io_slice(&self) -> impl Iterator<Item = IoSlice> {
        std::iter::empty()
    }
}

/// A trait for mutable vectored buffers.
pub trait IoVectoredBufMut: IoVectoredBuf + SetBufInit {
    /// An iterator for the [`IoSliceMut`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn io_slices_mut(&mut self) -> Vec<IoSliceMut> {
        self.iter_io_slice_mut().collect()
    }

    /// An iterator over the [`IoSliceMut`]s.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut>;

    /// An iterator over mutable slices.
    fn iter_slice_mut(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        unsafe {
            self.iter_io_slice_mut()
                .map(|slice| std::slice::from_raw_parts_mut(slice.as_ptr().cast(), slice.len()))
        }
    }
}

impl<T: IoBufMut> IoVectoredBufMut for &'static mut [T] {
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        self.iter_mut().map(|buf| buf.as_io_slice_mut())
    }
}

impl<T: IoBufMut, const N: usize> IoVectoredBufMut for [T; N] {
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        self.iter_mut().map(|buf| buf.as_io_slice_mut())
    }
}

impl<T: IoBufMut, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static>
    IoVectoredBufMut for t_alloc!(Vec, T, A)
{
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        self.iter_mut().map(|buf| buf.as_io_slice_mut())
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for arrayvec::ArrayVec<T, N> {
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        self.iter_mut().map(|buf| buf.as_io_slice_mut())
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        self.iter_mut().map(|buf| buf.as_io_slice_mut())
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut> IoVectoredBufMut for (T, Rest) {
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        std::iter::once(self.0.as_io_slice_mut()).chain(self.1.iter_io_slice_mut())
    }
}

impl IoVectoredBufMut for () {
    unsafe fn iter_io_slice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        std::iter::empty()
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut> SetBufInit for (T, Rest) {
    unsafe fn set_buf_init(&mut self, len: usize) {
        let buf0_len = std::cmp::min(len, self.0.buf_capacity());

        self.0.set_buf_init(buf0_len);
        self.1.set_buf_init(len - buf0_len);
    }
}

impl SetBufInit for () {
    unsafe fn set_buf_init(&mut self, len: usize) {
        assert_eq!(
            len, 0,
            "set_buf_init called with non-zero len on empty buffer"
        );
    }
}

/// An owned iterator over a vectored buffer.
pub struct VectoredBufIter<T> {
    buf: T,
    index: usize,
    len: usize,
    filled: usize,
    cur_filled: usize,
}

impl<T> VectoredBufIter<T> {
    /// Create a new [`VectoredBufIter`] from an indexable container. If the
    /// container is empty, return the buffer back in `Err(T)`.
    pub fn next(mut self) -> Result<Self, T> {
        self.index += 1;
        if self.index < self.len {
            self.filled += self.cur_filled;
            self.cur_filled = 0;
            Ok(self)
        } else {
            Err(self.buf)
        }
    }
}

impl<T> IntoInner for VectoredBufIter<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buf
    }
}

unsafe impl<T: IoVectoredBuf> IoBuf for VectoredBufIter<T> {
    fn as_buf_ptr(&self) -> *const u8 {
        unsafe {
            self.buf
                .iter_slice()
                .nth(self.index)
                .unwrap()
                .as_ptr()
                .add(self.cur_filled)
        }
    }

    fn buf_len(&self) -> usize {
        self.buf.iter_slice().nth(self.index).unwrap().len() - self.cur_filled
    }

    fn buf_capacity(&self) -> usize {
        self.buf_len()
    }
}

impl<T: IoVectoredBuf + SetBufInit> SetBufInit for VectoredBufIter<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.cur_filled += len;
        self.buf.set_buf_init(self.filled + self.cur_filled);
    }
}

unsafe impl<T: IoVectoredBufMut> IoBufMut for VectoredBufIter<T> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        unsafe {
            self.buf
                .iter_slice_mut()
                .nth(self.index)
                .unwrap()
                .as_mut_ptr()
                .add(self.cur_filled)
                .cast()
        }
    }
}
