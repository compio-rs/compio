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
    end: usize,
}

impl<T> Slice<T> {
    pub(crate) fn new(buffer: T, begin: usize, end: usize) -> Self {
        Self { buffer, begin, end }
    }

    /// Offset in the underlying buffer at which this slice starts.
    pub fn begin(&self) -> usize {
        self.begin
    }

    /// Offset in the underlying buffer at which this slice ends.
    pub fn end(&self) -> usize {
        self.end
    }

    pub(crate) fn set_range(&mut self, begin: usize, end: usize) {
        self.begin = begin;
        self.end = end;
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

fn slice_mut<T: IoBufMut>(buffer: &mut T) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(buffer.as_buf_mut_ptr(), (*buffer).buf_len()) }
}

impl<T: IoBuf> Deref for Slice<T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let bytes = self.buffer.as_slice();
        let end = self.end.min(bytes.len());
        &bytes[self.begin..end]
    }
}

impl<T: IoBufMut> DerefMut for Slice<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let bytes = slice_mut(&mut self.buffer);
        let end = self.end.min(bytes.len());
        &mut bytes[self.begin..end]
    }
}

unsafe impl<T: IoBuf> IoBuf for Slice<T> {
    fn as_buf_ptr(&self) -> *const u8 {
        self.buffer.as_slice()[self.begin..].as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.deref().len()
    }

    fn buf_capacity(&self) -> usize {
        self.end - self.begin
    }
}

unsafe impl<T: IoBufMut> IoBufMut for Slice<T> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        slice_mut(&mut self.buffer)[self.begin..].as_mut_ptr()
    }
}

impl<T: SetBufInit> SetBufInit for Slice<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.buffer.set_buf_init(self.begin + len)
    }
}

impl<T> IntoInner for Slice<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// An owned view of a vectored buffer.
pub struct VectoredSlice<T> {
    buf: T,
    begin: usize,
}

impl<T> VectoredSlice<T> {
    pub(crate) fn new(buf: T, begin: usize) -> Self {
        Self { buf, begin }
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
    unsafe fn iter_io_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        let mut offset = self.begin;
        self.buf.iter_io_buffer().filter_map(move |buf| {
            let len = buf.len();
            let sub = len.min(offset);
            offset -= sub;
            if len - sub > 0 {
                Some(IoBuffer::new(
                    buf.as_ptr().add(sub),
                    len - sub,
                    buf.capacity() - sub,
                ))
            } else {
                None
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

/// An owned view of a vectored buffer.
pub struct VectoredSliceMut<T> {
    buf: T,
    begin: usize,
}

impl<T> VectoredSliceMut<T> {
    pub(crate) fn new(buf: T, begin: usize) -> Self {
        Self { buf, begin }
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

impl<T: IoVectoredBuf> IoVectoredBuf for VectoredSliceMut<T> {
    unsafe fn iter_io_buffer(&self) -> impl Iterator<Item = IoBuffer> {
        let mut offset = self.begin;
        self.buf.iter_io_buffer().filter_map(move |buf| {
            let capacity = buf.capacity();
            let sub = capacity.min(offset);
            offset -= sub;
            if capacity - sub > 0 {
                let len = buf.len().saturating_sub(sub);
                Some(IoBuffer::new(buf.as_ptr().add(sub), len, capacity - sub))
            } else {
                None
            }
        })
    }
}

impl<T> IntoInner for VectoredSliceMut<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buf
    }
}

impl<T: SetBufInit> SetBufInit for VectoredSliceMut<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.buf.set_buf_init(self.begin + len);
    }
}

impl<T: IoVectoredBufMut> IoVectoredBufMut for VectoredSliceMut<T> {}
