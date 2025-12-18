use std::{iter, mem::MaybeUninit};

use crate::{IntoInner, IoBuf, IoBufMut, SetLen, VectoredSlice, t_alloc};

/// A trait for vectored buffers.
///
/// # Note for implementors
///
/// The iterator must be idemptotent and always yield the same slices in the
/// exact same orders, i.e., [`Iterator::enumerate`] will mark the same buffer
/// with same index.
pub trait IoVectoredBuf: 'static {
    /// An iterator of initialized slice of the buffers.
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]>;

    /// The total length of all buffers.
    fn total_len(&self) -> usize {
        self.iter_slice().map(|buf| buf.len()).sum()
    }

    /// Wrap self into an owned iterator.
    fn owned_iter(self) -> Result<VectoredBufIter<Self>, Self>
    where
        Self: Sized,
    {
        VectoredBufIter::new(self)
    }

    /// Get an owned view of the vectored buffer that skips the first
    /// `begin`-many **initialized** bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_buf::{IoBuf, IoVectoredBuf, VectoredSlice};
    ///
    /// # fn main() {
    /// /// Create a buffer with given content and capacity.
    /// fn new_buf(slice: &[u8], cap: usize) -> Vec<u8> {
    ///     let mut buf = Vec::new();
    ///     buf.reserve_exact(cap);
    ///     buf.extend_from_slice(slice);
    ///     buf
    /// }
    ///
    /// let bufs = [new_buf(b"hello", 10), new_buf(b"world", 10)];
    /// let vectored_buf = bufs.slice(3);
    /// let mut iter = vectored_buf.iter_slice();
    /// let buf1 = iter.next().unwrap();
    /// let buf2 = iter.next().unwrap();
    /// assert_eq!(&buf1.as_slice()[..], b"lo");
    /// assert_eq!(&buf2.as_slice()[..], b"world");
    ///
    /// let bufs = [new_buf(b"hello", 10), new_buf(b"world", 10)];
    /// let vectored_buf = bufs.slice(6);
    /// let mut iter = vectored_buf.iter_slice();
    /// let buf1 = iter.next().unwrap();
    /// assert!(iter.next().is_none());
    /// assert_eq!(&buf1.as_slice()[..], b"orld");
    /// # }
    /// ```
    fn slice(self, begin: usize) -> VectoredSlice<Self>
    where
        Self: Sized,
    {
        let mut offset = begin;
        let mut idx = 0;

        for b in self.iter_slice() {
            let len = b.len();
            if len > offset {
                break;
            }
            offset -= len;
            idx += 1;
        }

        VectoredSlice::new(self, begin, idx, offset)
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static [T] {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.iter().map(|buf| buf.as_slice())
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static mut [T] {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.iter().map(|buf| buf.as_slice())
    }
}

impl<T: IoBuf, const N: usize> IoVectoredBuf for [T; N] {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.iter().map(|buf| buf.as_slice())
    }
}

impl<T: IoBuf, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static> IoVectoredBuf
    for t_alloc!(Vec, T, A)
{
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.iter().map(|buf| buf.as_slice())
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for arrayvec::ArrayVec<T, N> {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.iter().map(|buf| buf.as_slice())
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        self.iter().map(|buf| buf.as_slice())
    }
}

impl<T: IoBuf, Rest: IoVectoredBuf> IoVectoredBuf for (T, Rest) {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        std::iter::once(self.0.as_slice()).chain(self.1.iter_slice())
    }
}

impl<T: IoBuf> IoVectoredBuf for (T,) {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        std::iter::once(self.0.as_slice())
    }
}

impl IoVectoredBuf for () {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        std::iter::empty()
    }
}

/// A trait for mutable vectored buffers.
pub trait IoVectoredBufMut: IoVectoredBuf + SetLen {
    /// An iterator of maybe uninitialized slice of the buffers.
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]>;

    /// The total capacity of all buffers.
    fn total_capacity(&mut self) -> usize {
        self.iter_uninit_slice().map(|buf| buf.len()).sum()
    }

    /// Get an owned view of the vectored buffer.
    ///
    /// Unlike [`IoVectoredBuf::slice`], the iterator returned by this function
    /// will skip both initialized and uninitialized bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_buf::{IoBuf, IoVectoredBuf, IoVectoredBufMut, VectoredSlice};
    ///
    /// # fn main() {
    /// /// Create a buffer with given content and capacity.
    /// fn new_buf(slice: &[u8], cap: usize) -> Vec<u8> {
    ///     let mut buf = Vec::new();
    ///     buf.reserve_exact(cap);
    ///     buf.extend_from_slice(slice);
    ///     buf
    /// }
    ///
    /// let bufs = [new_buf(b"hello", 10), new_buf(b"world", 10)];
    /// let vectored_buf = bufs.slice_mut(13);
    /// let mut iter = vectored_buf.iter_slice();
    /// let buf1 = iter.next().unwrap();
    /// assert!(iter.next().is_none());
    /// assert_eq!(buf1.as_slice(), b"ld");
    /// # }
    /// ```
    fn slice_mut(mut self, begin: usize) -> VectoredSlice<Self>
    where
        Self: Sized,
    {
        let mut offset = begin;
        let mut idx = 0;

        for b in self.iter_uninit_slice() {
            let len = b.len();
            if len > offset {
                break;
            }
            offset -= len;
            idx += 1;
        }

        VectoredSlice::new(self, begin, idx, offset)
    }
}

impl<T: IoBufMut> IoVectoredBufMut for &'static mut [T] {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        self.iter_mut().map(|buf| buf.as_uninit())
    }
}

impl<T: IoBufMut, const N: usize> IoVectoredBufMut for [T; N] {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        self.iter_mut().map(|buf| buf.as_uninit())
    }
}

impl<T: IoBufMut, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static>
    IoVectoredBufMut for t_alloc!(Vec, T, A)
{
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        self.iter_mut().map(|buf| buf.as_uninit())
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for arrayvec::ArrayVec<T, N> {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        self.iter_mut().map(|buf| buf.as_uninit())
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        self.iter_mut().map(|buf| buf.as_uninit())
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut> IoVectoredBufMut for (T, Rest) {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        let (h, t) = self;
        iter::once(h.as_uninit()).chain(t.iter_uninit_slice())
    }
}

impl<T: IoBufMut> IoVectoredBufMut for (T,) {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        iter::once(self.0.as_uninit())
    }
}

impl IoVectoredBufMut for () {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        iter::empty()
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut> SetLen for (T, Rest) {
    unsafe fn set_len(&mut self, len: usize) {
        let head_len = std::cmp::min(len, self.0.buf_capacity());
        let rest_len = len - head_len;

        // SAFETY: head_len <= self.0.buf_capacity()
        unsafe { self.0.set_len(head_len) };
        // SAFETY: propagate
        unsafe { self.1.set_len(rest_len) };
    }
}

impl<T: IoBufMut> SetLen for (T,) {
    unsafe fn set_len(&mut self, len: usize) {
        unsafe { self.0.set_len(len) };
    }
}

impl SetLen for () {
    unsafe fn set_len(&mut self, len: usize) {
        assert_eq!(len, 0, "set_len called with non-zero len on empty buffer");
    }
}

/// An owned iterator over a vectored buffer.
///
/// Normally one would use [`IoVectoredBuf::owned_iter`] to create this
/// iterator.
pub struct VectoredBufIter<T> {
    buf: T,
    total_filled: usize,
    index: usize,
    len: usize,
    filled: usize,
}

impl<T> VectoredBufIter<T> {
    /// Create a new [`VectoredBufIter`] from an indexable container. If the
    /// container is empty, return the buffer back in `Err(T)`.
    pub fn next(mut self) -> Result<Self, T> {
        self.index += 1;
        if self.index < self.len {
            self.total_filled += self.filled;
            self.filled = 0;
            Ok(self)
        } else {
            Err(self.buf)
        }
    }
}

impl<T: IoVectoredBuf> VectoredBufIter<T> {
    fn new(buf: T) -> Result<Self, T> {
        let len = buf.iter_slice().count();
        if len > 0 {
            Ok(Self {
                buf,
                index: 0,
                len,
                total_filled: 0,
                filled: 0,
            })
        } else {
            Err(buf)
        }
    }
}

impl<T> IntoInner for VectoredBufIter<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buf
    }
}

impl<T: IoVectoredBuf> IoBuf for VectoredBufIter<T> {
    fn as_slice(&self) -> &[u8] {
        let curr = self
            .buf
            .iter_slice()
            .nth(self.index)
            .expect("`index` should not exceed `len`");

        &curr[self.filled..]
    }
}

impl<T: IoVectoredBuf + SetLen> SetLen for VectoredBufIter<T> {
    unsafe fn set_len(&mut self, len: usize) {
        self.filled = len;

        unsafe { self.buf.set_len(self.total_filled + self.filled) };
    }
}

impl<T: IoVectoredBufMut> IoBufMut for VectoredBufIter<T> {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        self.buf
            .iter_uninit_slice()
            .nth(self.index)
            .expect("`index` should not exceed `len`")
    }
}
