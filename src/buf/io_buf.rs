#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::mem::MaybeUninit;

use crate::{buf::*, vec_alloc, vec_alloc_lifetime};

/// An IOCP compatible buffer.
///
/// The `IoBuf` trait is implemented by buffer types that can be passed to
/// IOCP operations. Users will not need to use this trait directly.
///
/// # Safety
///
/// Buffers passed to IOCP operations must reference a stable memory
/// region. While the runtime holds ownership to a buffer, the pointer returned
/// by `as_buf_ptr` must remain valid even if the `IoBuf` value is moved.
pub unsafe trait IoBuf<'arena>: 'arena {
    /// Returns a raw pointer to the vector’s buffer.
    ///
    /// This method is to be used by the `compio` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// The implementation must ensure that, while the `compio` runtime
    /// owns the value, the pointer returned **does not** change.
    fn as_buf_ptr(&self) -> *const u8;

    /// Number of initialized bytes.
    ///
    /// This method is to be used by the `compio` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// For [`Vec`], this is identical to `len()`.
    fn buf_len(&self) -> usize;

    /// Total size of the buffer, including uninitialized memory, if any.
    ///
    /// This method is to be used by the `compio` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// For [`Vec`], this is identical to `capacity()`.
    fn buf_capacity(&self) -> usize;

    /// Get the initialized part of the buffer.
    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.as_buf_ptr(), self.buf_len()) }
    }

    /// Returns a view of the buffer with the specified range.
    ///
    /// This method is similar to Rust's slicing (`&buf[..]`), but takes
    /// ownership of the buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio::buf::IoBuf;
    ///
    /// let buf = b"hello world";
    /// buf.slice(5..10);
    /// ```
    fn slice(self, range: impl std::ops::RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        use std::ops::Bound;

        let begin = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

        assert!(begin < self.buf_capacity());

        let end = match range.end_bound() {
            Bound::Included(&n) => n.checked_add(1).expect("out of range"),
            Bound::Excluded(&n) => n,
            Bound::Unbounded => self.buf_capacity(),
        };

        assert!(end <= self.buf_capacity());
        assert!(begin <= self.buf_len());

        Slice::new(self, begin, end)
    }
}

unsafe impl<#[cfg(feature = "allocator_api")] 'a, A: Allocator + Unpin + vec_alloc_lifetime!()>
    IoBuf<vec_alloc_lifetime!()> for vec_alloc!(u8, A)
{
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.capacity()
    }
}

unsafe impl IoBuf for &'static mut [u8] {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.len()
    }
}

unsafe impl<'a> IoBuf<'a> for &'a [u8] {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.len()
    }
}

unsafe impl IoBuf<'static> for String {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.capacity()
    }
}

unsafe impl<'a> IoBuf<'a> for &'a mut str {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.len()
    }
}

unsafe impl<'a> IoBuf<'a> for &'a str {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.len()
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBuf<'static> for bytes::Bytes {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.len()
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBuf<'static> for bytes::BytesMut {
    fn as_buf_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.capacity()
    }
}

#[cfg(feature = "read_buf")]
unsafe impl IoBuf<'arena> for std::io::BorrowedBuf<'arena> {
    fn as_buf_ptr(&self) -> *const u8 {
        self.filled().as_ptr()
    }

    fn buf_len(&self) -> usize {
        self.len()
    }

    fn buf_capacity(&self) -> usize {
        self.capacity()
    }
}

/// A mutable IOCP compatible buffer.
///
/// The `IoBufMut` trait is implemented by buffer types that can be passed to
/// IOCP operations. Users will not need to use this trait directly.
///
/// # Safety
///
/// Buffers passed to IOCP operations must reference a stable memory
/// region. While the runtime holds ownership to a buffer, the pointer returned
/// by `as_buf_mut_ptr` must remain valid even if the `IoBufMut` value is moved.
pub unsafe trait IoBufMut<'arena>: IoBuf<'arena> {
    /// Returns a raw mutable pointer to the vector’s buffer.
    ///
    /// This method is to be used by the `compio` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// The implementation must ensure that, while the `compio` runtime
    /// owns the value, the pointer returned **does not** change.
    fn as_buf_mut_ptr(&mut self) -> *mut u8;

    /// Get the uninitialized part of the buffer.
    fn as_uninit_slice(&mut self) -> &mut [MaybeUninit<u8>] {
        unsafe {
            std::slice::from_raw_parts_mut(
                self.as_buf_mut_ptr().add(self.buf_len()) as _,
                self.buf_capacity() - self.buf_len(),
            )
        }
    }

    /// Updates the number of initialized bytes.
    ///
    /// The specified `len` plus [`IoBuf::buf_len`] becomes the new value
    /// returned by [`IoBuf::buf_len`].
    fn set_buf_init(&mut self, len: usize);
}

unsafe impl<#[cfg(feature = "allocator_api")] A: Allocator + Unpin + vec_alloc_lifetime!()> IoBufMut<vec_alloc_lifetime!()>
    for vec_alloc!(u8, A)
{
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn set_buf_init(&mut self, len: usize) {
        unsafe { self.set_len(len + self.buf_len()) };
    }
}

unsafe impl<a> IoBufMut<a> for &'a mut [u8] {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn set_buf_init(&mut self, len: usize) {
        debug_assert!(len == 0)
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBufMut<'static> for bytes::BytesMut {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn set_buf_init(&mut self, len: usize) {
        unsafe { self.set_len(len + self.buf_len()) };
    }
}

#[cfg(feature = "read_buf")]
unsafe impl IoBufMut<'arena> for std::io::BorrowedBuf<'arena> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.filled().as_ptr() as _
    }

    fn set_buf_init(&mut self, len: usize) {
        unsafe { self.unfilled().advance(len) };
    }
}
