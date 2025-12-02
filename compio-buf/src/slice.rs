use std::ops::{Deref, DerefMut};

use crate::*;

/// An owned view into a contiguous sequence of bytes.
///
/// This is similar to Rust slices (`&buf[..]`) but owns the underlying buffer.
/// This type is useful for performing io-uring read and write operations using
/// a subset of a buffer.
///
/// Slices are created using [`IoBuf::slice`].
///
/// # Examples
///
/// Creating a slice
///
/// ```
/// use compio_buf::IoBuf;
///
/// let buf = b"hello world";
/// let slice = buf.slice(..5);
///
/// assert_eq!(&slice[..], b"hello");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slice<T> {
    buffer: T,
    begin: usize,
    end: Option<usize>,
}

impl<T> Slice<T> {
    pub(crate) fn new(buffer: T, begin: usize, end: Option<usize>) -> Self {
        Self { buffer, begin, end }
    }

    /// Offset in the underlying buffer at which this slice starts.
    pub fn begin(&self) -> usize {
        self.begin
    }

    /// Offset in the underlying buffer at which this slice ends.
    pub fn end(&self) -> Option<usize> {
        self.end
    }

    /// Gets a reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner(&self) -> &T {
        &self.buffer
    }

    /// Gets a mutable reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.buffer
    }
}

impl<T: IoBuf> Slice<T> {
    /// Offset in the underlying buffer at which this slice starts. If it
    /// exceeds the buffer length, returns the buffer length.
    fn begin_or_len(&self) -> usize {
        let len = self.buffer.buf_len();
        self.begin.min(len)
    }

    /// Offset in the underlying buffer at which this slice ends. If it does not
    /// exist or exceeds the buffer length, returns the buffer length.
    fn end_or_len(&self) -> usize {
        let len = self.buffer.buf_len();
        self.end.unwrap_or(len).min(len)
    }
}

impl<T: IoBufMut> Slice<T> {
    /// Offset in the underlying buffer at which this slice ends. If it does not
    /// exist or exceeds the buffer capacity, returns the buffer capacity.
    fn end_or_cap(&self) -> usize {
        let cap = self.buffer.buf_capacity();
        self.end.unwrap_or(cap).min(cap)
    }
}

impl<T: IoBuf> Deref for Slice<T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let bytes = self.buffer.as_slice();
        let begin = self.begin_or_len();
        let end = self.end_or_len();
        &bytes[begin..end]
    }
}

impl<T: IoBufMut> DerefMut for Slice<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let ptr = self.buffer.as_buf_ptr();
        let begin = self.begin_or_len();
        let end = self.end_or_len();
        unsafe { std::slice::from_raw_parts_mut(ptr.add(begin) as _, end - begin) }
    }
}

unsafe impl<T: IoBuf> IoBuf for Slice<T> {
    unsafe fn buffer(&self) -> IoBuffer {
        // SAFETY: The slice is bounded by &self, as required.
        unsafe { IoBuffer::from_slice(self.deref()) }
    }
}

unsafe impl<T: IoBufMut> IoBufMut for Slice<T> {
    fn uninit_len(&self) -> usize {
        let begin = self.begin().max(self.buffer.buf_len());
        let end = self.end_or_cap();
        end - begin
    }
}

impl<T: SetBufInit> SetBufInit for Slice<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { self.buffer.set_buf_init(self.begin + len) }
    }
}

impl<T> IntoInner for Slice<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Type for [`IoVectoredBuf::slice`].
pub struct VectoredSlice<T> {
    buf: T,
    begin: usize,
    idx: usize,
    offset: usize,
}

impl<T: IoVectoredBuf> VectoredSlice<T> {
    pub(crate) fn new(buf: T, begin: usize) -> Self {
        let mut offset = begin;
        let mut idx = 0;

        for b in unsafe { buf.iter_buffer() } {
            let len = b.len();
            if len > offset {
                break;
            }
            offset -= len;
            idx += 1;
        }

        Self {
            buf,
            begin,
            idx,
            offset,
        }
    }

    /// Offset in the underlying buffer at which this slice starts.
    pub fn begin(&self) -> usize {
        self.begin
    }

    /// Gets a reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner(&self) -> &T {
        &self.buf
    }

    /// Gets a mutable reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.buf
    }
}

impl<T: IoVectoredBuf> IoVectoredBuf for VectoredSlice<T> {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        unsafe { self.buf.iter_buffer() }
            .skip(self.idx)
            .enumerate()
            .map(move |(idx, buf)| {
                if idx != 0 {
                    buf
                } else {
                    unsafe { IoBuffer::from_slice(&buf.slice()[self.offset..]) }
                }
            })
    }
}

impl<T> IntoInner for VectoredSlice<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buf
    }
}

impl<T: SetBufInit> SetBufInit for VectoredSlice<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { self.buf.set_buf_init(self.begin + len) }
    }
}

impl<T> IntoInner for VectoredSliceMut<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buf
    }
}

impl<T: IoVectoredBufMut> IoVectoredBufMut for VectoredSlice<T> {
    fn uninit_len_of(&self, idx: usize) -> usize {
        self.buf.uninit_len_of(idx + self.idx)
    }

    fn capacity_of(&self, idx: usize) -> usize {
        self.buf.capacity_of(idx + self.idx)
    }
}

/// Type for mutable slice of [`IoVectoredBufMut`].
pub struct VectoredSliceMut<T> {
    buf: T,
    begin: usize,
    idx: usize,
    offset: usize,
}

/// Type for [`IoVectoredBufMut::slice_mut`].
impl<T: IoVectoredBufMut> VectoredSliceMut<T> {
    pub(crate) fn new(buf: T, begin: usize) -> Self {
        let mut offset = begin;
        let mut idx = 0;

        loop {
            let cap = buf.capacity_of(idx);
            if cap > offset {
                break;
            }
            offset -= cap;
            idx += 1;
        }

        Self {
            buf,
            begin,
            idx,
            offset,
        }
    }

    /// Offset in the underlying buffer at which this slice starts.
    pub fn begin(&self) -> usize {
        self.begin
    }

    /// Gets a reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner(&self) -> &T {
        &self.buf
    }

    /// Gets a mutable reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.buf
    }
}

impl<T: IoVectoredBufMut> IoVectoredBuf for VectoredSliceMut<T> {
    unsafe fn iter_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        unsafe { self.buf.iter_buffer() }
            .skip(self.idx)
            .enumerate()
            .map(move |(idx, buf)| {
                if idx != 0 {
                    buf
                } else {
                    let begin = self.offset.min(buf.len());
                    unsafe { IoBuffer::from_slice(&buf.slice()[begin..]) }
                }
            })
    }
}

impl<T: SetBufInit> SetBufInit for VectoredSliceMut<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { self.buf.set_buf_init(self.begin + len) }
    }
}

impl<T: IoVectoredBufMut> IoVectoredBufMut for VectoredSliceMut<T> {
    fn uninit_len_of(&self, idx: usize) -> usize {
        // Only the first buffer is affected by the offset.
        if idx == 0 {
            let buf = unsafe { self.buf.iter_buffer() }
                .nth(self.idx)
                .expect("index out of bounds");
            // It's possible for offset to go beyond buf.len().
            self.buf.capacity_of(self.idx) - buf.len().max(self.offset)
        } else {
            self.buf.uninit_len_of(idx + self.idx)
        }
    }

    fn capacity_of(&self, idx: usize) -> usize {
        if idx == 0 {
            self.buf.capacity_of(self.idx) - self.offset
        } else {
            self.buf.capacity_of(idx + self.idx)
        }
    }
}

#[test]
fn test_slice() {
    let buf = b"hello world";
    let slice = buf.slice(6..);
    assert_eq!(slice.as_slice(), b"world");

    let slice = buf.slice(..5);
    assert_eq!(slice.as_slice(), b"hello");

    let slice = buf.slice(3..8);
    assert_eq!(slice.as_slice(), b"lo wo");

    let slice = buf.slice(..);
    assert_eq!(slice.as_slice(), b"hello world");

    let slice = buf.slice(12..);
    assert_eq!(slice.as_slice(), b"");
}
