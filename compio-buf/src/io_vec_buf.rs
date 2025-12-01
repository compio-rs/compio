use std::mem::MaybeUninit;

use crate::{
    IntoInner, IoBuf, IoBufMut, IoBuffer, IoBufferMut, SetBufInit, VectoredSlice, VectoredSliceMut,
    t_alloc,
};

/// A trait for vectored buffers.
pub trait IoVectoredBuf: 'static {
    /// An iterator over the [`IoBuffer`]s. It is static to provide convenience
    /// from writing self-referenced structure.
    ///
    /// # Safety
    ///
    /// All safety requirement for `IoBuffer` must be held.
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer>;

    /// An iterator over slices.
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        unsafe { self.iter_buffer().map(|buf| buf.slice()) }
    }

    /// The total length of all buffers.
    fn total_len(&self) -> usize {
        unsafe { self.iter_buffer().map(|buf| buf.len()).sum() }
    }

    /// Wrap self into an owned iterator.
    fn owned_iter(self) -> Result<VectoredBufIter<Self>, Self>
    where
        Self: Sized,
    {
        VectoredBufIter::new(self)
    }

    /// Get an owned view of the vectored buffer that only counts in initialized
    /// bytes. Uninitialized bytes are ignored.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_buf::{IoBuffer, IoVectoredBuf, VectoredSlice};
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
    /// let mut iter = unsafe { vectored_buf.iter_buffer() };
    /// let buf1 = iter.next().unwrap();
    /// let buf2 = iter.next().unwrap();
    /// assert_eq!(&unsafe { buf1.slice() }[..], b"lo");
    /// assert_eq!(&unsafe { buf2.slice() }[..], b"world");
    ///
    /// let bufs = [new_buf(b"hello", 10), new_buf(b"world", 10)];
    /// let vectored_buf = bufs.slice(6);
    /// let mut iter = unsafe { vectored_buf.iter_buffer() };
    /// let buf1 = iter.next().unwrap();
    /// assert!(iter.next().is_none());
    /// assert_eq!(&unsafe { buf1.slice() }[..], b"orld");
    /// # }
    /// ```
    fn slice(self, begin: usize) -> VectoredSlice<Self>
    where
        Self: Sized,
    {
        VectoredSlice::new(self, begin)
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static [T] {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        self.iter().map(|buf| unsafe { buf.buffer() })
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static mut [T] {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        self.iter().map(|buf| unsafe { buf.buffer() })
    }
}

impl<T: IoBuf, const N: usize> IoVectoredBuf for [T; N] {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        self.iter().map(|buf| unsafe { buf.buffer() })
    }
}

impl<T: IoBuf, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static> IoVectoredBuf
    for t_alloc!(Vec, T, A)
{
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        self.iter().map(|buf| unsafe { buf.buffer() })
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for arrayvec::ArrayVec<T, N> {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        self.iter().map(|buf| unsafe { buf.buffer() })
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        self.iter().map(|buf| unsafe { buf.buffer() })
    }
}

impl<T: IoBuf, Rest: IoVectoredBuf> IoVectoredBuf for (T, Rest) {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        unsafe { std::iter::once(self.0.buffer()).chain(self.1.iter_buffer()) }
    }
}

impl<T: IoBuf> IoVectoredBuf for (T,) {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        unsafe { std::iter::once(self.0.buffer()) }
    }
}

impl IoVectoredBuf for () {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        std::iter::empty()
    }
}

/// A trait for mutable vectored buffers.
pub trait IoVectoredBufMut: IoVectoredBuf + SetBufInit {
    /// Get the uninitialized length of the buffer at the given index.
    fn uninit_len_of(&self, idx: usize) -> usize;

    /// Get the capacity of the buffer at the given index.
    fn capacity_of(&self, idx: usize) -> usize {
        unsafe {
            let buf = self
                .iter_buffer()
                .nth(idx)
                .expect("index out of bounds in capacity_of");
            buf.len() + self.uninit_len_of(idx)
        }
    }
    /// An iterator for the [`IoSliceMut`]s of the buffers. It is static to
    /// provide convenience from writing self-referenced structure.
    ///
    /// # Safety
    ///
    /// All safety requirement for `IoBufferMut` must be held.
    unsafe fn iter_buffer_mut(&mut self) -> impl Iterator<Item = IoBufferMut> {
        unsafe {
            self.iter_buffer().enumerate().map(|(idx, buf)| {
                let (ptr, len) = buf.into_piece();
                let uninit_len = self.uninit_len_of(idx);
                IoBufferMut::new(ptr as *mut MaybeUninit<u8>, len + uninit_len)
            })
        }
    }

    /// The total capacity of all buffers.
    fn total_capacity(&mut self) -> usize {
        unsafe { self.iter_buffer_mut().map(|buf| buf.len()).sum() }
    }

    /// An iterator over mutable slices.
    fn iter_slice_mut(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        unsafe { self.iter_buffer_mut().map(|slice| slice.slice_mut()) }
    }

    /// Get an owned view of the vectored buffer.
    ///
    /// Unlike [`IoVectoredBuf::slice`], the iterator returned by this function
    /// will count in both initialized and uninitialized bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_buf::{IoBuffer, IoVectoredBuf, IoVectoredBufMut, VectoredSlice};
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
    /// let mut iter = unsafe { vectored_buf.iter_buffer() };
    /// let buf1 = iter.next().unwrap();
    /// assert!(iter.next().is_none());
    /// assert_eq!(&unsafe { buf1.slice() }[..], b"ld");
    /// # }
    /// ```
    fn slice_mut(self, begin: usize) -> VectoredSliceMut<Self>
    where
        Self: Sized,
    {
        VectoredSliceMut::new(self, begin)
    }
}

impl<T: IoBufMut> IoVectoredBufMut for &'static mut [T] {
    fn uninit_len_of(&self, idx: usize) -> usize {
        self[idx].uninit_len()
    }

    fn capacity_of(&self, idx: usize) -> usize {
        self[idx].buf_capacity()
    }
}

impl<T: IoBufMut, const N: usize> IoVectoredBufMut for [T; N] {
    fn uninit_len_of(&self, idx: usize) -> usize {
        self[idx].uninit_len()
    }

    fn capacity_of(&self, idx: usize) -> usize {
        self[idx].buf_capacity()
    }
}

impl<T: IoBufMut, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static>
    IoVectoredBufMut for t_alloc!(Vec, T, A)
{
    fn uninit_len_of(&self, idx: usize) -> usize {
        self[idx].uninit_len()
    }

    fn capacity_of(&self, idx: usize) -> usize {
        self[idx].buf_capacity()
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for arrayvec::ArrayVec<T, N> {
    fn uninit_len_of(&self, idx: usize) -> usize {
        self[idx].uninit_len()
    }

    fn capacity_of(&self, idx: usize) -> usize {
        self[idx].buf_capacity()
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    fn uninit_len_of(&self, idx: usize) -> usize {
        self[idx].uninit_len()
    }

    fn capacity_of(&self, idx: usize) -> usize {
        self[idx].buf_capacity()
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut> IoVectoredBufMut for (T, Rest) {
    fn uninit_len_of(&self, idx: usize) -> usize {
        if idx == 0 {
            self.0.uninit_len()
        } else {
            self.1.uninit_len_of(idx - 1)
        }
    }
}

impl<T: IoBufMut> IoVectoredBufMut for (T,) {
    fn uninit_len_of(&self, idx: usize) -> usize {
        debug_assert!(idx == 0);
        self.0.uninit_len()
    }

    fn capacity_of(&self, idx: usize) -> usize {
        debug_assert!(idx == 0);
        self.0.buf_capacity()
    }
}

impl IoVectoredBufMut for () {
    fn uninit_len_of(&self, _: usize) -> usize {
        unreachable!("no buffers in empty tuple")
    }

    fn capacity_of(&self, _: usize) -> usize {
        unreachable!("no buffers in empty tuple")
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut> SetBufInit for (T, Rest) {
    unsafe fn set_buf_init(&mut self, len: usize) {
        let buf0_len = std::cmp::min(len, self.0.buf_capacity());

        // SAFETY: buf0_len <= self.0.buf_capacity()
        unsafe { self.0.set_buf_init(buf0_len) };
        // SAFETY: propagate
        unsafe { self.1.set_buf_init(len - buf0_len) };
    }
}

impl<T: IoBufMut> SetBufInit for (T,) {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { self.0.set_buf_init(len) };
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
    total_filled: usize,
    cur_filled: usize,
}

impl<T> VectoredBufIter<T> {
    /// Create a new [`VectoredBufIter`] from an indexable container. If the
    /// container is empty, return the buffer back in `Err(T)`.
    pub fn next(mut self) -> Result<Self, T> {
        self.index += 1;
        if self.index < self.len {
            self.total_filled += self.cur_filled;
            self.cur_filled = 0;
            Ok(self)
        } else {
            Err(self.buf)
        }
    }
}

impl<T: IoVectoredBuf> VectoredBufIter<T> {
    fn new(buf: T) -> Result<Self, T> {
        let len = unsafe { buf.iter_buffer().count() };
        if len > 0 {
            Ok(Self {
                buf,
                index: 0,
                len,
                total_filled: 0,
                cur_filled: 0,
            })
        } else {
            Err(buf)
        }
    }

    fn current(&self) -> IoBuffer {
        unsafe {
            self.buf
                .iter_buffer()
                .nth(self.index)
                .expect("`index` should not exceed `len`")
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
    unsafe fn buffer(&self) -> IoBuffer {
        let slice = unsafe { &self.current().slice()[self.cur_filled..] };
        unsafe { IoBuffer::from_slice(slice) }
    }
}

impl<T: IoVectoredBuf + SetBufInit> SetBufInit for VectoredBufIter<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.cur_filled = len;
        unsafe { self.buf.set_buf_init(self.total_filled + self.cur_filled) };
    }
}

unsafe impl<T: IoVectoredBufMut> IoBufMut for VectoredBufIter<T> {
    fn uninit_len(&self) -> usize {
        self.buf.uninit_len_of(self.index)
    }
}
