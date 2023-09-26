#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
};

use crate::*;

/// A trait for buffers.
///
/// The `IoBuf` trait is implemented by buffer types that can be passed to
/// compio operations. Users will not need to use this trait directly.
///
/// # Safety
///
/// Buffers passed to compio operations must refer to a stable memory region.
/// While the runtime holds ownership to a buffer, the pointer returned
/// by `as_buf_ptr` must remain valid even if the `IoBuf` value is moved, i.e.,
/// the type implementing `IoBuf` should point to somewhere else.
pub unsafe trait IoBuf: Unpin + 'static {
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

    #[doc(hidden)]
    unsafe fn as_io_slice(&self) -> IoSlice<'static> {
        IoSlice::new(std::mem::transmute(self.as_slice()))
    }

    /// Returns a view of the buffer with the specified range.
    ///
    /// This method is similar to Rust's slicing (`&buf[..]`), but takes
    /// ownership of the buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_buf::IoBuf;
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

unsafe impl<#[cfg(feature = "allocator_api")] A: Allocator + Unpin + 'static> IoBuf
    for vec_alloc!(u8, A)
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

unsafe impl IoBuf for &'static [u8] {
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

unsafe impl IoBuf for String {
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

unsafe impl IoBuf for &'static mut str {
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

unsafe impl IoBuf for &'static str {
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
unsafe impl IoBuf for bytes::Bytes {
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
unsafe impl IoBuf for bytes::BytesMut {
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
unsafe impl IoBuf for std::io::BorrowedBuf<'static> {
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

#[cfg(feature = "arrayvec")]
unsafe impl<const N: usize> IoBuf for arrayvec::ArrayVec<u8, N> {
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

/// A mutable compio compatible buffer.
///
/// The `IoBufMut` trait is implemented by buffer types that can be passed to
/// compio operations. Users will not need to use this trait directly.
///
/// # Safety
///
/// Buffers passed to compio operations must reference a stable memory
/// region. While the runtime holds ownership to a buffer, the pointer returned
/// by `as_buf_mut_ptr` must remain valid even if the `IoBufMut` value is moved.
pub unsafe trait IoBufMut: IoBuf {
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

    #[doc(hidden)]
    unsafe fn as_io_slice_mut(&mut self) -> IoSliceMut<'static> {
        IoSliceMut::new(std::mem::transmute(self.as_uninit_slice()))
    }
}

unsafe impl<#[cfg(feature = "allocator_api")] A: Allocator + Unpin + 'static> IoBufMut
    for vec_alloc!(u8, A)
{
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

unsafe impl IoBufMut for &'static mut [u8] {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBufMut for bytes::BytesMut {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

#[cfg(feature = "read_buf")]
unsafe impl IoBufMut for std::io::BorrowedBuf<'static> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.filled().as_ptr() as _
    }
}

#[cfg(feature = "arrayvec")]
unsafe impl<const N: usize> IoBufMut for arrayvec::ArrayVec<u8, N> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

/// A helper trait for `set_len` like methods.
pub trait SetBufInit {
    /// Updates the number of initialized bytes.
    ///
    /// The specified `len` plus [`IoBuf::buf_len`] becomes the new value
    /// returned by [`IoBuf::buf_len`].
    ///
    /// # Safety
    ///
    /// `len` should be less or equal than `buf_capacity() - buf_len()`.
    unsafe fn set_buf_init(&mut self, len: usize);
}

impl<T: IoBuf + SetBufInit> SetBufInit for Vec<T> {
    unsafe fn set_buf_init(&mut self, mut len: usize) {
        for buf in self {
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

impl<#[cfg(feature = "allocator_api")] A: Allocator + Unpin + 'static> SetBufInit
    for vec_alloc!(u8, A)
{
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.set_len(len + self.buf_len());
    }
}

#[cfg(feature = "bytes")]
impl SetBufInit for bytes::BytesMut {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.set_len(len + self.buf_len());
    }
}

#[cfg(feature = "read_buf")]
impl SetBufInit for std::io::BorrowedBuf<'static> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.unfilled().advance(len);
    }
}

#[cfg(feature = "arrayvec")]
impl<const N: usize> SetBufInit for arrayvec::ArrayVec<u8, N> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.set_len(len + self.buf_len());
    }
}

#[doc(hidden)]
pub unsafe trait AsIoSlices {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn as_io_slices(&self) -> Vec<IoSlice<'static>>;
}

unsafe impl<T: IoBuf> AsIoSlices for Vec<T> {
    unsafe fn as_io_slices(&self) -> Vec<IoSlice<'static>> {
        self.iter()
            .map(|buf| IoSlice::new(std::mem::transmute(buf.as_slice())))
            .collect()
    }
}

#[doc(hidden)]
pub unsafe trait AsIoSlicesMut: AsIoSlices {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn as_io_slices_mut(&mut self) -> Vec<IoSliceMut<'static>>;
}

unsafe impl<T: IoBufMut> AsIoSlicesMut for Vec<T> {
    unsafe fn as_io_slices_mut(&mut self) -> Vec<IoSliceMut<'static>> {
        self.iter_mut()
            .map(|buf| IoSliceMut::new(std::mem::transmute(buf.as_uninit_slice())))
            .collect()
    }
}
