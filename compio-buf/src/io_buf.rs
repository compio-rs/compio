#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::{mem::MaybeUninit, rc::Rc, sync::Arc};

use crate::*;

/// A trait for immutable buffers.
///
/// The `IoBuf` trait is implemented by buffer types that can be passed to
/// immutable completion-based IO operations, like writing its content to a
/// file. This trait will only take initialized bytes of a buffer into account.
///
/// # Safety
///
/// The implementer must ensure that they return a valid [`IoBuffer`] in
/// [`IoBuf::buffer`]. For detail, see safety section in [`IoBuffer::new`].
pub unsafe trait IoBuf: 'static {
    /// Create an [`IoBuffer`] for this buffer. It is static to provide
    /// convenience from writing self-referenced structure.
    ///
    /// # Safety
    ///
    /// The returned [`IoBuffer`] must not outlive `&self`.
    unsafe fn buffer(&self) -> IoBuffer;

    /// Length of initialized bytes in the buffer.
    fn buf_len(&self) -> usize {
        // SAFETY: We're only using the length of the buffer here.
        unsafe { self.buffer().len() }
    }

    /// Get the raw pointer to the buffer.
    fn as_buf_ptr(&self) -> *const u8 {
        // SAFETY: We're only taking the pointer here.
        let buf = unsafe { self.buffer() };
        buf.as_ptr()
    }

    /// Get an immutable slice of the buffer.
    fn as_slice(&self) -> &[u8] {
        let (ptr, len) = unsafe { self.buffer().into_piece() };
        // SAFETY:
        // - returned slice is bounded by &self
        // - &[u8] guarantees to be initialized
        unsafe { std::slice::from_raw_parts(ptr, len) }
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
    /// assert_eq!(buf.slice(6..).as_slice(), b"world");
    /// ```
    fn slice(self, range: impl std::ops::RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        use std::ops::Bound;

        let len = self.buf_len();

        let begin = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

        assert!(begin <= len);

        let end = match range.end_bound() {
            Bound::Included(&n) => Some(n.checked_add(1).expect("out of range")),
            Bound::Excluded(&n) => Some(n),
            Bound::Unbounded => None,
        };

        if let Some(end) = end {
            assert!(begin <= end);
        }

        Slice::new(self, begin, end)
    }
}

unsafe impl<B: IoBuf + ?Sized> IoBuf for &'static B {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

unsafe impl<B: IoBuf + ?Sized> IoBuf for &'static mut B {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

unsafe impl<B: IoBuf + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Box, B, A)
{
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

unsafe impl<B: IoBuf + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Rc, B, A)
{
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

unsafe impl IoBuf for [u8] {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { IoBuffer::from_slice(self) }
    }
}

unsafe impl<const N: usize> IoBuf for [u8; N] {
    unsafe fn buffer(&self) -> IoBuffer {
        // Safety: [u8; N] always holds initialized bytes within `N`.
        unsafe { IoBuffer::new(self.as_ptr(), N) }
    }
}

unsafe impl<#[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Vec, u8, A)
{
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

unsafe impl IoBuf for str {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { IoBuffer::from_slice(self.as_bytes()) }
    }
}

unsafe impl IoBuf for String {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { self.as_str().buffer() }
    }
}

unsafe impl<B: IoBuf + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Arc, B, A)
{
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBuf for bytes::Bytes {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBuf for bytes::BytesMut {
    unsafe fn buffer(&self) -> IoBuffer {
        unsafe { (**self).buffer() }
    }
}

#[cfg(feature = "read_buf")]
unsafe impl IoBuf for std::io::BorrowedBuf<'static> {
    unsafe fn buffer(&self) -> IoBuffer {
        let filled = (*self).is_filled();
        // Safety: filled part of BorrowedBuf is always initialized.
        unsafe { IoBuffer::new(filled.as_ptr(), filled.len()) }
    }
}

#[cfg(feature = "arrayvec")]
unsafe impl<const N: usize> IoBuf for arrayvec::ArrayVec<u8, N> {
    unsafe fn buffer(&self) -> IoBuffer {
        // Safety: ArrayVec<_, N> always holds initialized bytes within `N`.
        unsafe { IoBuffer::new(self.as_ptr(), N) }
    }
}

#[cfg(feature = "smallvec")]
unsafe impl<const N: usize> IoBuf for smallvec::SmallVec<[u8; N]>
where
    [u8; N]: smallvec::Array<Item = u8>,
{
    unsafe fn buffer(&self) -> IoBuffer {
        // Safety: SmallVec<[_; N]> always holds initialized bytes within `N`.
        unsafe { IoBuffer::new(self.as_ptr(), N) }
    }
}

/// A trait for mutable buffers.
///
/// The `IoBufMut` trait is implemented by buffer types that can be passed to
/// mutable completion-based IO operations, like reading content from a file and
/// write to the buffer. This trait will take all space of a buffer into
/// account, including uninitialized bytes.
///
/// # Safety
///
/// The implementer must ensure `(self.buffer().ptr, self.buffer().len() +
///   uninit_len)` is a valid `&mut [MaybeUninit<u8>]`, that is: the
/// `uninit_len`-long region after `self.buffer` is accessible for writes,
/// despite being uninitialized.
pub unsafe trait IoBufMut: IoBuf + SetBufInit {
    /// Length of accessible uninitialized bytes following the initialized
    /// bytes.
    fn uninit_len(&self) -> usize;

    /// Total capacity of the buffer, including both initialized and
    /// uninitialized bytes.
    fn buf_capacity(&self) -> usize {
        self.buf_len() + self.uninit_len()
    }

    /// Get the raw mutable pointer to the buffer.
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        let buf = unsafe { self.buffer_mut() };
        buf.as_ptr() as _
    }

    /// Create an [`IoBufferMut`] for this buffer. It is static to provide
    /// convenience from writing self-referenced structure.
    ///
    /// # Safety
    ///
    /// The returned [`IoBufferMut`] must not outlive `&self`.
    unsafe fn buffer_mut(&mut self) -> IoBufferMut {
        let (ptr, len) = unsafe { (*self).buffer().into_piece() };
        let uninit_len = (*self).uninit_len();
        // SAFETY:
        // - `buf.ptr` is valid for `buf.len() + uninit_len` bytes.
        // - the pointer is not used for anything else while the `IoBufferMut` is in
        //   use, guranteed by `&mut self` and `IoBufMut`'s short-live safety contract.
        unsafe { IoBufferMut::new(ptr as _, len + uninit_len) }
    }

    /// Get a mutable slice of the whole buffer, including uninitialized
    /// bytes.
    fn as_mut_slice(&mut self) -> &mut [MaybeUninit<u8>] {
        let buf = unsafe { self.buffer_mut() };
        // SAFETY: returned slice is bounded by &mut self
        unsafe { std::slice::from_raw_parts_mut(buf.as_ptr(), buf.len()) }
    }

    /// Returns an [`Uninit`], which is a [`Slice`] that only exposes
    /// uninitialized bytes.
    ///
    /// It will always point to the uninitialized area of a [`IoBufMut`] even
    /// after reading in some bytes, which is done by [`SetBufInit`]. This
    /// is useful for writing data into buffer without overwriting any
    /// existing bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_buf::{IoBuf, IoBufMut};
    ///
    /// let mut buf = Vec::from(b"hello world");
    /// buf.reserve_exact(10);
    /// let slice = buf.uninit();
    ///
    /// assert_eq!(slice.as_slice(), b"");
    /// assert_eq!(slice.buf_capacity(), 10);
    /// ```
    fn uninit(self) -> Uninit<Self>
    where
        Self: Sized,
    {
        Uninit::new(self)
    }

    /// Indicate whether the buffer has been filled (uninit portion is empty)
    fn is_filled(&mut self) -> bool {
        let len = (*self).buf_len();
        let cap = (*self).buf_capacity();
        len == cap
    }
}

unsafe impl<B: IoBufMut + ?Sized> IoBufMut for &'static mut B {
    fn uninit_len(&self) -> usize {
        (**self).uninit_len()
    }
}

unsafe impl<B: IoBufMut + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBufMut
    for t_alloc!(Box, B, A)
{
    fn uninit_len(&self) -> usize {
        (**self).uninit_len()
    }
}

unsafe impl<#[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBufMut
    for t_alloc!(Vec, u8, A)
{
    fn uninit_len(&self) -> usize {
        self.capacity() - self.len()
    }
}

unsafe impl IoBufMut for [u8] {
    fn uninit_len(&self) -> usize {
        0
    }
}

unsafe impl<const N: usize> IoBufMut for [u8; N] {
    fn uninit_len(&self) -> usize {
        0
    }
}

#[cfg(feature = "bytes")]
unsafe impl IoBufMut for bytes::BytesMut {
    fn uninit_len(&self) -> usize {
        (**self).uninit_len()
    }
}

#[cfg(feature = "read_buf")]
unsafe impl IoBufMut for std::io::BorrowedBuf<'static> {
    fn uninit_len(&self) -> usize {
        self.capacity() - self.len()
    }
}

#[cfg(feature = "arrayvec")]
unsafe impl<const N: usize> IoBufMut for arrayvec::ArrayVec<u8, N> {
    fn uninit_len(&self) -> usize {
        self.capacity() - self.len()
    }
}

#[cfg(feature = "smallvec")]
unsafe impl<const N: usize> IoBufMut for smallvec::SmallVec<[u8; N]>
where
    [u8; N]: smallvec::Array<Item = u8>,
{
    fn uninit_len(&self) -> usize {
        self.capacity() - self.len()
    }
}

/// A helper trait for `set_len` like methods.
pub trait SetBufInit {
    /// Set the buffer length. If `len` is less than the current length, this
    /// operation must be a no-op.
    ///
    /// # Safety
    ///
    /// * `len` must be less or equal than `buffer_mut().len()`.
    /// * The bytes in the range `[buf_len(), len)` must be initialized.
    unsafe fn set_buf_init(&mut self, len: usize);
}

impl<B: SetBufInit + ?Sized> SetBufInit for &'static mut B {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { (**self).set_buf_init(len) }
    }
}

impl<B: SetBufInit + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> SetBufInit
    for t_alloc!(Box, B, A)
{
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { (**self).set_buf_init(len) }
    }
}

impl<#[cfg(feature = "allocator_api")] A: Allocator + 'static> SetBufInit for t_alloc!(Vec, u8, A) {
    unsafe fn set_buf_init(&mut self, len: usize) {
        if (**self).buf_len() < len {
            unsafe { self.set_len(len) };
        }
    }
}

impl SetBufInit for [u8] {
    unsafe fn set_buf_init(&mut self, len: usize) {
        debug_assert!(len <= self.len());
    }
}

impl<const N: usize> SetBufInit for [u8; N] {
    unsafe fn set_buf_init(&mut self, len: usize) {
        debug_assert!(len <= N);
    }
}

#[cfg(feature = "bytes")]
impl SetBufInit for bytes::BytesMut {
    unsafe fn set_buf_init(&mut self, len: usize) {
        if (**self).buf_len() < len {
            unsafe { self.set_len(len) };
        }
    }
}

#[cfg(feature = "read_buf")]
impl SetBufInit for std::io::BorrowedBuf<'static> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        let current_len = (*self).buf_len();
        if current_len < len {
            self.unfilled().advance(len - current_len);
        }
    }
}

#[cfg(feature = "arrayvec")]
impl<const N: usize> SetBufInit for arrayvec::ArrayVec<u8, N> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        if (**self).buf_len() < len {
            unsafe { self.set_len(len) };
        }
    }
}

#[cfg(feature = "smallvec")]
impl<const N: usize> SetBufInit for smallvec::SmallVec<[u8; N]>
where
    [u8; N]: smallvec::Array<Item = u8>,
{
    unsafe fn set_buf_init(&mut self, len: usize) {
        if (**self).buf_len() < len {
            unsafe { self.set_len(len) };
        }
    }
}

impl<T: IoBufMut> SetBufInit for [T] {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { default_set_buf_init(self.iter_mut(), len) }
    }
}

impl<T: IoBufMut, const N: usize> SetBufInit for [T; N] {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { default_set_buf_init(self.iter_mut(), len) }
    }
}

impl<T: IoBufMut, #[cfg(feature = "allocator_api")] A: Allocator + 'static> SetBufInit
    for t_alloc!(Vec, T, A)
{
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { default_set_buf_init(self.iter_mut(), len) }
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBufMut, const N: usize> SetBufInit for arrayvec::ArrayVec<T, N> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { default_set_buf_init(self.iter_mut(), len) }
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBufMut, const N: usize> SetBufInit for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe { default_set_buf_init(self.iter_mut(), len) }
    }
}

/// # Safety
/// * `len` should be less or equal than the sum of `buf_capacity()` of all
///   buffers.
/// * The bytes in the range `[buf_len(), new_len)` of each buffer must be
///   initialized
unsafe fn default_set_buf_init<'a, B: IoBufMut>(
    iter: impl IntoIterator<Item = &'a mut B>,
    mut len: usize,
) {
    for buf in iter {
        let capacity = (*buf).buf_capacity();
        if len >= capacity {
            unsafe { buf.set_buf_init(capacity) };
            len -= capacity;
        } else {
            unsafe { buf.set_buf_init(len) };
            len = 0;
        }
    }
}
