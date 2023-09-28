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

fn deref<T: IoBuf>(buffer: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(buffer.as_buf_ptr(), buffer.buf_len()) }
}

fn deref_mut<T: IoBufMut>(buffer: &mut T) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(buffer.as_buf_mut_ptr(), buffer.buf_len()) }
}

impl<T: IoBuf> Deref for Slice<T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let bytes = deref(&self.buffer);
        let end = self.end.min(bytes.len());
        &bytes[self.begin..end]
    }
}

impl<T: IoBufMut> DerefMut for Slice<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let bytes = deref_mut(&mut self.buffer);
        let end = self.end.min(bytes.len());
        &mut bytes[self.begin..end]
    }
}

unsafe impl<T: IoBuf> IoBuf for Slice<T> {
    fn as_buf_ptr(&self) -> *const u8 {
        deref(&self.buffer)[self.begin..].as_ptr()
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
        deref_mut(&mut self.buffer)[self.begin..].as_mut_ptr()
    }
}

impl<T: SetBufInit> SetBufInit for Slice<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len)
    }
}

impl<T> IntoInner for Slice<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
