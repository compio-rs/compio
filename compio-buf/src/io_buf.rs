#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::{error::Error, fmt::Display, mem::MaybeUninit, rc::Rc, sync::Arc};

use crate::*;

/// A trait for immutable buffers.
///
/// The `IoBuf` trait is implemented by buffer types that can be passed to
/// immutable completion-based IO operations, like writing its content to a
/// file. This trait will only take initialized bytes of a buffer into account.
pub trait IoBuf: 'static {
    /// Get the slice of initialized bytes.
    fn as_slice(&self) -> &[u8];

    /// Length of initialized bytes in the buffer.
    fn buf_len(&self) -> usize {
        self.as_slice().len()
    }

    /// Raw pointer to the buffer.
    fn buf_ptr(&self) -> *const u8 {
        self.as_slice().as_ptr()
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

        let begin = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

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

impl<B: IoBuf + ?Sized> IoBuf for &'static B {
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

impl<B: IoBuf + ?Sized> IoBuf for &'static mut B {
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

impl<B: IoBuf + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Box, B, A)
{
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

impl<B: IoBuf + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Rc, B, A)
{
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

impl IoBuf for [u8] {
    fn as_slice(&self) -> &[u8] {
        self
    }
}

impl<const N: usize> IoBuf for [u8; N] {
    fn as_slice(&self) -> &[u8] {
        self
    }
}

impl<#[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf for t_alloc!(Vec, u8, A) {
    fn as_slice(&self) -> &[u8] {
        Vec::as_slice(self)
    }
}

impl IoBuf for str {
    fn as_slice(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl IoBuf for String {
    fn as_slice(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<B: IoBuf + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBuf
    for t_alloc!(Arc, B, A)
{
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

#[cfg(feature = "bytes")]
impl IoBuf for bytes::Bytes {
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

#[cfg(feature = "bytes")]
impl IoBuf for bytes::BytesMut {
    fn as_slice(&self) -> &[u8] {
        (**self).as_slice()
    }
}

#[cfg(feature = "read_buf")]
impl IoBuf for std::io::BorrowedBuf<'static> {
    fn as_slice(&self) -> &[u8] {
        self.filled()
    }
}

#[cfg(feature = "arrayvec")]
impl<const N: usize> IoBuf for arrayvec::ArrayVec<u8, N> {
    fn as_slice(&self) -> &[u8] {
        self
    }
}

#[cfg(feature = "smallvec")]
impl<const N: usize> IoBuf for smallvec::SmallVec<[u8; N]>
where
    [u8; N]: smallvec::Array<Item = u8>,
{
    fn as_slice(&self) -> &[u8] {
        self
    }
}

/// An error indicating that reserving capacity for a buffer failed.
#[derive(Debug)]
pub enum ReserveError {
    /// Reservation is not supported.
    NotSupported,

    /// Reservation failed.
    ReserveFailed(Box<dyn Error + Send + Sync>),
}

impl ReserveError {
    /// Check if the error is `NotSupported`.
    pub fn is_not_supported(&self) -> bool {
        matches!(self, ReserveError::NotSupported)
    }
}

impl Display for ReserveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReserveError::NotSupported => write!(f, "reservation is not supported"),
            ReserveError::ReserveFailed(src) => write!(f, "reservation failed: {src}"),
        }
    }
}

impl Error for ReserveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ReserveError::ReserveFailed(src) => Some(src.as_ref()),
            _ => None,
        }
    }
}

/// An error indicating that reserving exact capacity for a buffer failed.
#[derive(Debug)]
pub enum ReserveExactError {
    /// Reservation is not supported.
    NotSupported,

    /// Reservation failed.
    ReserveFailed(Box<dyn Error + Send + Sync>),

    /// Reserved size does not match the expected size.
    ExactSizeMismatch {
        /// Expected size to reserve
        expected: usize,

        /// Actual size reserved
        reserved: usize,
    },
}

impl ReserveExactError {
    /// Check if the error is `NotSupported`.
    pub fn is_not_supported(&self) -> bool {
        matches!(self, ReserveExactError::NotSupported)
    }
}

impl Display for ReserveExactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReserveExactError::NotSupported => write!(f, "reservation is not supported"),
            ReserveExactError::ReserveFailed(src) => write!(f, "reservation failed: {src}"),
            ReserveExactError::ExactSizeMismatch { reserved, expected } => {
                write!(
                    f,
                    "reserved size mismatch: expected {}, reserved {}",
                    expected, reserved
                )
            }
        }
    }
}

impl From<ReserveError> for ReserveExactError {
    fn from(err: ReserveError) -> Self {
        match err {
            ReserveError::NotSupported => ReserveExactError::NotSupported,
            ReserveError::ReserveFailed(src) => ReserveExactError::ReserveFailed(src),
        }
    }
}

impl Error for ReserveExactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ReserveExactError::ReserveFailed(src) => Some(src.as_ref()),
            _ => None,
        }
    }
}

#[cfg(feature = "smallvec")]
mod smallvec_err {
    use std::{error::Error, fmt::Display};

    use smallvec::CollectionAllocErr;

    #[derive(Debug)]
    pub(super) struct SmallVecErr(pub CollectionAllocErr);

    impl Display for SmallVecErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "SmallVec allocation error: {}", self.0)
        }
    }

    impl Error for SmallVecErr {}
}

/// A trait for mutable buffers.
///
/// The `IoBufMut` trait is implemented by buffer types that can be passed to
/// mutable completion-based IO operations, like reading content from a file and
/// write to the buffer. This trait will take all space of a buffer into
/// account, including uninitialized bytes.
pub trait IoBufMut: IoBuf {
    /// Get the full mutable slice of the buffer, including both initialized
    /// and uninitialized bytes.
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>];

    /// Total capacity of the buffer, including both initialized and
    /// uninitialized bytes.
    fn buf_capacity(&mut self) -> usize {
        self.as_uninit().len()
    }

    /// Get the raw mutable pointer to the buffer.
    fn buf_mut_ptr(&mut self) -> *mut MaybeUninit<u8> {
        (*self).as_slice().as_ptr() as _
    }

    /// Get the mutable slice of initialized bytes. The content is the same as
    /// `as_slice`, but mutable.
    fn as_mut_slice(&mut self) -> &mut [u8] {
        let len = (*self).buf_len();
        let ptr = (*self).buf_mut_ptr();
        // SAFETY:
        // - lifetime of the returned slice is bounded by &mut self
        // - bytes within `len` are guaranteed to be initialized
        // - the pointer is derived from
        unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, len) }
    }

    /// Reserve additional capacity for the buffer.
    ///
    /// This is a no-op by default. Types that support dynamic resizing
    /// (like `Vec<u8>`) will override this method to actually reserve
    /// capacity. The return value indicates whether the reservation succeeded.
    /// See [`ReserveError`] for details.
    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        let _ = len;
        Err(ReserveError::NotSupported)
    }

    /// Reserve exactly `len` additional capacity for the buffer.
    ///
    /// By default this falls back to [`IoBufMut::reserve`], which is a no-op
    /// for most types. Types that support dynamic resizing (like `Vec<u8>`)
    /// will override this method to actually reserve capacity. The return value
    /// indicates whether the exact reservation succeeded. See
    /// [`ReserveExactError`] for details.
    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        self.reserve(len)?;
        Ok(())
    }

    /// Returns an [`Uninit`], which is a [`Slice`] that only exposes
    /// uninitialized bytes.
    ///
    /// It will always point to the uninitialized area of a [`IoBufMut`] even
    /// after reading in some bytes, which is done by [`SetLen`]. This
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
    /// let mut slice = buf.uninit();
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
        let len = (*self).as_slice().len();
        let cap = (*self).buf_capacity();
        len == cap
    }
}

impl<B: IoBufMut + ?Sized> IoBufMut for &'static mut B {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        (**self).as_uninit()
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        (**self).reserve(len)
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        (**self).reserve_exact(len)
    }
}

impl<B: IoBufMut + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBufMut
    for t_alloc!(Box, B, A)
{
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        (**self).as_uninit()
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        (**self).reserve(len)
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        (**self).reserve_exact(len)
    }
}

impl<#[cfg(feature = "allocator_api")] A: Allocator + 'static> IoBufMut for t_alloc!(Vec, u8, A) {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let ptr = self.as_mut_ptr() as *mut MaybeUninit<u8>;
        let cap = self.capacity();
        // SAFETY: Vec guarantees that the pointer is valid for `capacity` bytes
        unsafe { std::slice::from_raw_parts_mut(ptr, cap) }
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        if let Err(e) = Vec::try_reserve(self, len) {
            return Err(ReserveError::ReserveFailed(Box::new(e)));
        }

        Ok(())
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        if self.capacity() - self.len() >= len {
            return Ok(());
        }

        if let Err(e) = Vec::try_reserve_exact(self, len) {
            return Err(ReserveExactError::ReserveFailed(Box::new(e)));
        }

        if self.capacity() - self.len() != len {
            return Err(ReserveExactError::ExactSizeMismatch {
                reserved: self.capacity() - self.len(),
                expected: len,
            });
        }
        Ok(())
    }
}

impl IoBufMut for [u8] {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let ptr = self.as_mut_ptr() as *mut MaybeUninit<u8>;
        let len = self.len();
        // SAFETY: slice is fully initialized, so treating it as MaybeUninit is safe
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
    }
}

impl<const N: usize> IoBufMut for [u8; N] {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let ptr = self.as_mut_ptr() as *mut MaybeUninit<u8>;
        // SAFETY: array is fully initialized, so treating it as MaybeUninit is safe
        unsafe { std::slice::from_raw_parts_mut(ptr, N) }
    }
}

#[cfg(feature = "bytes")]
impl IoBufMut for bytes::BytesMut {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let ptr = self.as_mut_ptr() as *mut MaybeUninit<u8>;
        let cap = self.capacity();
        // SAFETY: BytesMut guarantees that the pointer is valid for `capacity` bytes
        unsafe { std::slice::from_raw_parts_mut(ptr, cap) }
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        bytes::BytesMut::reserve(self, len);
        Ok(())
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        if self.capacity() - self.len() >= len {
            return Ok(());
        }

        bytes::BytesMut::reserve(self, len);

        if self.capacity() - self.len() != len {
            Err(ReserveExactError::ExactSizeMismatch {
                reserved: self.capacity() - self.len(),
                expected: len,
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(feature = "read_buf")]
impl IoBufMut for std::io::BorrowedBuf<'static> {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let total_cap = self.capacity();

        // SAFETY: We reconstruct the full buffer from the filled portion pointer.
        // BorrowedBuf guarantees that the underlying buffer has capacity bytes.
        unsafe {
            let filled_ptr = self.filled().as_ptr() as *mut MaybeUninit<u8>;
            std::slice::from_raw_parts_mut(filled_ptr, total_cap)
        }
    }
}

#[cfg(feature = "arrayvec")]
impl<const N: usize> IoBufMut for arrayvec::ArrayVec<u8, N> {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let ptr = self.as_mut_ptr() as *mut MaybeUninit<u8>;
        // SAFETY: ArrayVec guarantees that the pointer is valid for N bytes
        unsafe { std::slice::from_raw_parts_mut(ptr, N) }
    }
}

#[cfg(feature = "smallvec")]
impl<const N: usize> IoBufMut for smallvec::SmallVec<[u8; N]>
where
    [u8; N]: smallvec::Array<Item = u8>,
{
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let ptr = self.as_mut_ptr() as *mut MaybeUninit<u8>;
        let cap = self.capacity();
        // SAFETY: SmallVec guarantees that the pointer is valid for `capacity` bytes
        unsafe { std::slice::from_raw_parts_mut(ptr, cap) }
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        if let Err(e) = smallvec::SmallVec::try_reserve(self, len) {
            return Err(ReserveError::ReserveFailed(Box::new(
                smallvec_err::SmallVecErr(e),
            )));
        }
        Ok(())
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        if self.capacity() - self.len() >= len {
            return Ok(());
        }

        if let Err(e) = smallvec::SmallVec::try_reserve_exact(self, len) {
            return Err(ReserveExactError::ReserveFailed(Box::new(
                smallvec_err::SmallVecErr(e),
            )));
        }

        if self.capacity() - self.len() != len {
            return Err(ReserveExactError::ExactSizeMismatch {
                reserved: self.capacity() - self.len(),
                expected: len,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::IoBufMut;

    #[test]
    fn test_vec_reserve() {
        let mut buf = Vec::new();
        IoBufMut::reserve(&mut buf, 10).unwrap();
        assert!(buf.capacity() >= 10);

        let mut buf = Vec::new();
        IoBufMut::reserve_exact(&mut buf, 10).unwrap();
        assert!(buf.capacity() == 10);

        let mut buf = Box::new(Vec::new());
        IoBufMut::reserve_exact(&mut buf, 10).unwrap();
        assert!(buf.capacity() == 10);
    }

    #[test]
    #[cfg(feature = "bytes")]
    fn test_bytes_reserve() {
        let mut buf = bytes::BytesMut::new();
        IoBufMut::reserve(&mut buf, 10).unwrap();
        assert!(buf.capacity() >= 10);
    }

    #[test]
    #[cfg(feature = "smallvec")]
    fn test_smallvec_reserve() {
        let mut buf = smallvec::SmallVec::<[u8; 8]>::new();
        IoBufMut::reserve(&mut buf, 10).unwrap();
        assert!(buf.capacity() >= 10);
    }

    #[test]
    fn test_other_reserve() {
        let mut buf = [1, 1, 4, 5, 1, 4];
        let res = IoBufMut::reserve(&mut buf, 10);
        assert!(res.is_err_and(|x| x.is_not_supported()));
        assert!(buf.buf_capacity() == 6);
    }
}
