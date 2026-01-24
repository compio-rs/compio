use std::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

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
    /// # Safety
    /// begin <= buf_len
    pub(crate) unsafe fn new(buffer: T, begin: usize, end: Option<usize>) -> Self {
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

impl<T: IoBuf> Slice<Slice<T>> {
    /// Flatten nested slices into a single slice.
    pub fn flatten(self) -> Slice<T> {
        let large_begin = self.buffer.begin;
        let large_end = self.buffer.end;

        let new_begin = large_begin + self.begin;
        let new_end = match (self.end, large_end) {
            (Some(small_end), Some(large_end)) => Some((large_begin + small_end).min(large_end)),
            (Some(small_end), None) => Some(large_begin + small_end),
            (None, large_end) => large_end,
        };

        // Safety: inner.begin + outer.begin <= buf_len
        unsafe { Slice::new(self.buffer.buffer, new_begin, new_end) }
    }
}

impl<T: IoBuf> Slice<T> {
    /// Offset in the underlying buffer at which this slice ends. If it does not
    /// exist or exceeds the buffer length, returns the buffer length.
    fn end_or_len(&self) -> usize {
        let len = self.buffer.buf_len();
        self.end.unwrap_or(len).min(len)
    }

    /// Range of initialized bytes in the slice.
    fn initialized_range(&self) -> std::ops::Range<usize> {
        let end = self.end_or_len();
        self.begin..end
    }
}

impl<T: IoBufMut> Slice<T> {
    /// Offset in the underlying buffer at which this slice ends. If it does not
    /// exist or exceeds the buffer capacity, returns the buffer capacity.
    fn end_or_cap(&mut self) -> usize {
        let cap = self.buffer.buf_capacity();
        self.end.unwrap_or(cap).min(cap)
    }

    /// Full range of the slice, include uninitialized bytes.
    fn range(&mut self) -> std::ops::Range<usize> {
        let end = self.end_or_cap();
        self.begin..end
    }
}

impl<T: IoBuf> Deref for Slice<T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let range = self.initialized_range();
        let bytes = self.buffer.as_init();
        &bytes[range]
    }
}

impl<T: IoBufMut> DerefMut for Slice<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let range = self.initialized_range();
        let bytes = self.buffer.as_mut_slice();
        &mut bytes[range]
    }
}

impl<T: IoBuf> IoBuf for Slice<T> {
    fn as_init(&self) -> &[u8] {
        self.deref()
    }
}

impl<T: IoBufMut> IoBufMut for Slice<T> {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let range = self.range();
        let bytes = self.buffer.as_uninit();
        &mut bytes[range]
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        if self.end.is_some() {
            // Cannot reserve on a fixed-size slice
            Err(ReserveError::NotSupported)
        } else {
            self.buffer.reserve(len)
        }
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        if self.end.is_some() {
            // Cannot reserve on a fixed-size slice
            Err(ReserveExactError::NotSupported)
        } else {
            self.buffer.reserve_exact(len)
        }
    }
}

impl<T: SetLen> SetLen for Slice<T> {
    unsafe fn set_len(&mut self, len: usize) {
        unsafe { self.buffer.set_len(self.begin + len) }
    }
}

impl<T> IntoInner for Slice<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Return type for [`IoVectoredBuf::slice`] and
/// [`IoVectoredBufMut::slice_mut`].
///
/// # Behavior
///
/// When constructing the [`VectoredSlice`], it will first compute how
/// many buffers to skip. Imaging vectored buffers as concatenated slices, there
/// will be uninitialized "slots" in between. This slice type provides two
/// behaviors of how to skip through those slots, controlled by the marker type
/// `B`:
///
/// - [`IoVectoredBuf::slice`]: Ignore uninitialized slots, i.e., skip
///   `begin`-many **initialized** bytes.
/// - [`IoVectoredBufMut::slice_mut`]: Consider uninitialized slots, i.e., skip
///   `begin`-many bytes.
///
/// This will only affect how the slice is being constructed. The resulting
/// slice will always expose all of the remaining bytes, no matter initialized
/// or not (in particular, [`IoVectoredBufMut::iter_uninit_slice`]).
pub struct VectoredSlice<T> {
    buf: T,
    begin: usize,
    idx: usize,
    offset: usize,
}

impl<T> IntoInner for VectoredSlice<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buf
    }
}

impl<T> VectoredSlice<T> {
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

    pub(crate) fn new(buf: T, begin: usize, idx: usize, offset: usize) -> Self {
        Self {
            buf,
            begin,
            idx,
            offset,
        }
    }
}

impl<T: IoVectoredBuf> IoVectoredBuf for VectoredSlice<T> {
    fn iter_slice(&self) -> impl Iterator<Item = &[u8]> {
        let mut offset = self.offset;
        self.buf.iter_slice().skip(self.idx).map(move |buf| {
            let ret = &buf[offset..];
            offset = 0;
            ret
        })
    }
}

impl<T: SetLen> SetLen for VectoredSlice<T> {
    unsafe fn set_len(&mut self, len: usize) {
        unsafe { self.buf.set_len(self.begin + len) }
    }
}

impl<T: IoVectoredBufMut> IoVectoredBufMut for VectoredSlice<T> {
    fn iter_uninit_slice(&mut self) -> impl Iterator<Item = &mut [MaybeUninit<u8>]> {
        let mut offset = self.offset;
        self.buf.iter_uninit_slice().skip(self.idx).map(move |buf| {
            let ret = &mut buf[offset..];
            offset = 0;
            ret
        })
    }
}

#[test]
fn test_slice() {
    let buf = b"hello world";
    let slice = buf.slice(6..);
    assert_eq!(slice.as_init(), b"world");

    let slice = buf.slice(..5);
    assert_eq!(slice.as_init(), b"hello");

    let slice = buf.slice(3..8);
    assert_eq!(slice.as_init(), b"lo wo");

    let slice = buf.slice(..);
    assert_eq!(slice.as_init(), b"hello world");

    let slice = buf.slice(11..);
    assert_eq!(slice.as_init(), b"");
}
