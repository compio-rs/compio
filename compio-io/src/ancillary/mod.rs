//! Ancillary data (control message) support for connected streams.
//!
//! Ancillary messages are used to pass out-of-band information such as file
//! descriptors (Unix domain sockets), credentials, or kTLS record types.
//!
//! # Types
//!
//! - [`AncillaryBuf`]: A fixed-size, properly aligned stack buffer for
//!   ancillary messages.
//! - [`AncillaryBuilder`]: A builder for constructing ancillary messages into a
//!   [`AncillaryBuf`].
//! - [`AncillaryIter`]: An iterator over a buffer of ancillary messages.
//! - [`AncillaryRef`]: A reference to a single ancillary data entry.
//! - [`AncillaryData`]: Trait for types that can be encoded/decoded as
//!   ancillary data payloads.
//! - [`CodecError`]: Error type for encoding/decoding operations.
//!
//! # Traits
//!
//! - [`AsyncReadAncillary`]: read data together with ancillary data
//! - [`AsyncWriteAncillary`]: write data together with ancillary data
//!
//! # Functions
//!
//! - [`ancillary_space`]: Helper function to calculate ancillary message size
//!   for a type.
//!
//! # Modules
//!
//! - [`bytemuck_ext`]: Extension module for automatic [`AncillaryData`]
//!   implementation via bytemuck (requires `bytemuck` feature).

use std::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr,
};

use compio_buf::{IoBuf, IoBufMut, SetLen};

mod io;

pub use self::io::*;

#[cfg(feature = "bytemuck")]
pub mod bytemuck_ext;
mod sys;

/// Reference to an ancillary (control) message.
pub struct AncillaryRef<'a>(sys::CMsgRef<'a>);

impl AncillaryRef<'_> {
    /// Returns the level of the control message.
    pub fn level(&self) -> i32 {
        self.0.level()
    }

    /// Returns the type of the control message.
    pub fn ty(&self) -> i32 {
        self.0.ty()
    }

    /// Returns the length of the control message.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len() as _
    }

    /// Returns a copy of the data in the control message.
    pub fn data<T: AncillaryData>(&self) -> Result<T, CodecError> {
        self.0.decode_data()
    }
}

/// An iterator for ancillary (control) messages.
pub struct AncillaryIter<'a> {
    inner: sys::CMsgIter,
    buffer: &'a [u8],
}

impl<'a> AncillaryIter<'a> {
    /// Create [`AncillaryIter`] with the given buffer.
    ///
    /// # Panics
    ///
    /// This function will panic if the buffer is too short or not properly
    /// aligned.
    ///
    /// # Safety
    ///
    /// The buffer should contain valid control messages.
    pub unsafe fn new(buffer: &'a [u8]) -> Self {
        Self {
            inner: sys::CMsgIter::new(buffer.as_ptr(), buffer.len()),
            buffer,
        }
    }
}

impl<'a> Iterator for AncillaryIter<'a> {
    type Item = AncillaryRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let cmsg = self.inner.current(self.buffer.as_ptr());
            self.inner.next(self.buffer.as_ptr());
            cmsg.map(AncillaryRef)
        }
    }
}

/// Helper to construct ancillary (control) messages.
pub struct AncillaryBuilder<'a, B: ?Sized> {
    inner: sys::CMsgIter,
    buffer: &'a mut B,
}

impl<'a, B: IoBufMut + ?Sized> AncillaryBuilder<'a, B> {
    /// Create [`AncillaryBuilder`] with the given buffer. The buffer will be
    /// cleared on creation.
    ///
    /// # Panics
    ///
    /// This function will panic if the buffer is too short or not properly
    /// aligned.
    pub fn new(buffer: &'a mut B) -> Self {
        // SAFETY: always safe to make it empty.
        unsafe { buffer.set_len(0) };
        let slice = buffer.ensure_init();
        let inner = sys::CMsgIter::new(slice.as_ptr(), slice.len());
        Self { inner, buffer }
    }

    /// Append a control message into the buffer.
    pub fn push<T: AncillaryData>(
        &mut self,
        level: i32,
        ty: i32,
        value: &T,
    ) -> Result<(), CodecError> {
        if !self.inner.is_space_enough(T::SIZE) {
            return Err(CodecError::BufferTooSmall);
        }

        // SAFETY: method `new` guarantees the buffer is zeroed and properly aligned,
        // and we have checked the space.
        let mut cmsg = unsafe { self.inner.current_mut(self.buffer.buf_mut_ptr().cast()) }
            .expect("sufficient space");
        cmsg.set_level(level);
        cmsg.set_ty(ty);
        unsafe {
            self.buffer.advance(cmsg.encode_data(value)?);
        }

        unsafe { self.inner.next(self.buffer.buf_mut_ptr().cast()) };

        Ok(())
    }
}

/// A fixed-size, stack-allocated buffer for ancillary (control) messages.
///
/// Properly aligned for the platform's control message header type
/// (`cmsghdr` on Unix, `CMSGHDR` on Windows), so it can be passed directly
/// to [`AncillaryIter`] and [`AncillaryBuilder`].
pub struct AncillaryBuf<const N: usize> {
    inner: [u8; N],
    len: usize,
    _align: [sys::cmsghdr; 0],
}

impl<const N: usize> AncillaryBuf<N> {
    /// Create a new zeroed [`AncillaryBuf`].
    pub fn new() -> Self {
        Self {
            inner: [0u8; N],
            len: 0,
            _align: [],
        }
    }

    /// Create [`AncillaryBuilder`] with this buffer. The buffer will be zeroed
    /// on creation.
    ///
    /// # Panics
    ///
    /// This function will panic if this buffer is too short.
    pub fn builder(&mut self) -> AncillaryBuilder<'_, Self> {
        AncillaryBuilder::new(self)
    }
}

impl<const N: usize> Default for AncillaryBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> IoBuf for AncillaryBuf<N> {
    fn as_init(&self) -> &[u8] {
        &self.inner[..self.len]
    }
}

impl<const N: usize> SetLen for AncillaryBuf<N> {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= N);
        self.len = len;
    }
}

impl<const N: usize> IoBufMut for AncillaryBuf<N> {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        self.inner.as_uninit()
    }
}

impl<const N: usize> Deref for AncillaryBuf<N> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.inner[0..self.len]
    }
}

impl<const N: usize> DerefMut for AncillaryBuf<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner[0..self.len]
    }
}

/// Returns the buffer size required to hold one ancillary message carrying a
/// value of type `T`.
///
/// This is the platform-appropriate equivalent of `CMSG_SPACE(T::SIZE)` on
/// Unix or `WSA_CMSG_SPACE(T::SIZE)` on Windows, and can be used as a const
/// generic argument for [`AncillaryBuf`].
pub const fn ancillary_space<T: AncillaryData>() -> usize {
    // SAFETY: CMSG_SPACE is always safe
    #[allow(clippy::unnecessary_cast)]
    unsafe {
        sys::CMSG_SPACE(T::SIZE as _) as usize
    }
}

/// Error that can occur when encoding or decoding ancillary data.
#[derive(Debug)]
pub enum CodecError {
    /// The provided buffer is too small to hold the encoded data.
    BufferTooSmall,
    /// Another error occurred during encoding or decoding.
    Other(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl CodecError {
    /// Create a new [`CodecError::Other`] from any error type.
    pub fn other(error: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Other(error.into())
    }

    /// Attempt to downcast the error to a concrete type.
    ///
    /// Returns `Some(&T)` if the error is of type `T`, otherwise `None`.
    pub fn downcast_ref<T: std::error::Error + 'static>(&self) -> Option<&T> {
        match self {
            Self::Other(e) => e.downcast_ref(),
            _ => None,
        }
    }

    /// Attempt to downcast the error to a concrete type.
    ///
    /// Returns `Some(&mut T)` if the error is of type `T`, otherwise `None`.
    pub fn downcast_mut<T: std::error::Error + 'static>(&mut self) -> Option<&mut T> {
        match self {
            Self::Other(e) => e.downcast_mut(),
            _ => None,
        }
    }
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferTooSmall => write!(f, "buffer too small for encoding/decoding"),
            Self::Other(e) => write!(f, "codec error: {}", e),
        }
    }
}

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

/// Trait for types that can be encoded and decoded as ancillary data payloads.
///
/// This trait enables a type to be used as the data payload in control messages
/// (ancillary data). Types implementing this trait can be passed to
/// [`AncillaryBuilder::push`] and retrieved via [`AncillaryRef::data`].
///
/// # Built-in Implementations
///
/// This trait is implemented for the following platform-specific types:
///
/// - Unix: `libc::in_addr`, `libc::in_pktinfo`, `libc::in6_pktinfo`
/// - Windows: `IN_PKTINFO`, `IN6_PKTINFO`
///
/// When the `bytemuck` feature is enabled, this trait is also automatically
/// implemented for types that implement [`bytemuck_ext::BitwiseAncillaryData`]:
///
/// - Primitive types: `()`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`, `i8`,
///   `i16`, `i32`, `i64`, `i128`, `isize`, `f32`, `f64`
/// - Fixed-size arrays of the above types (up to size 512)
///
/// For custom types with the `bytemuck` feature enabled, you can implement
/// [`bytemuck_ext::BitwiseAncillaryData`] to automatically get
/// [`AncillaryData`] (see [`bytemuck_ext`] for details). Otherwise, you must
/// manually implement this trait with custom encoding/decoding logic.
///
/// # Example
///
/// ```
/// use std::mem::MaybeUninit;
///
/// use compio_io::ancillary::{AncillaryData, CodecError};
///
/// struct MyData {
///     value: u32,
/// }
///
/// impl AncillaryData for MyData {
///     const SIZE: usize = std::mem::size_of::<u32>();
///
///     fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
///         if buffer.len() < Self::SIZE {
///             return Err(CodecError::BufferTooSmall);
///         }
///         let bytes = self.value.to_ne_bytes();
///         for (i, &byte) in bytes.iter().enumerate() {
///             buffer[i] = MaybeUninit::new(byte);
///         }
///         Ok(())
///     }
///
///     fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
///         if buffer.len() < Self::SIZE {
///             return Err(CodecError::BufferTooSmall);
///         }
///         let mut bytes = [0u8; 4];
///         bytes.copy_from_slice(&buffer[..4]);
///         Ok(MyData {
///             value: u32::from_ne_bytes(bytes),
///         })
///     }
/// }
/// ```
pub trait AncillaryData: Sized {
    /// The size in bytes of the encoded representation.
    ///
    /// This defaults to `std::mem::size_of::<Self>()` but can be overridden
    /// for types with custom encoding.
    const SIZE: usize = std::mem::size_of::<Self>();

    /// Encode this value into the provided buffer.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::BufferTooSmall`] if the buffer is too small to
    /// hold the encoded data, or [`CodecError::Other`] for other encoding
    /// errors.
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError>;

    /// Decode a value from the provided buffer.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::BufferTooSmall`] if the buffer is too small,
    /// or [`CodecError::Other`] for other decoding errors.
    fn decode(buffer: &[u8]) -> Result<Self, CodecError>;
}

unsafe fn copy_to_bytes<T: AncillaryData>(
    src: &T,
    dest: &mut [MaybeUninit<u8>],
) -> Result<(), CodecError> {
    if dest.len() < T::SIZE {
        return Err(CodecError::BufferTooSmall);
    }
    unsafe {
        ptr::copy_nonoverlapping::<u8>(src as *const T as _, dest.as_mut_ptr() as _, T::SIZE);
    }
    Ok(())
}

unsafe fn copy_from_bytes<T: AncillaryData>(src: &[u8]) -> Result<T, CodecError> {
    if src.len() < T::SIZE {
        return Err(CodecError::BufferTooSmall);
    }
    let src_ptr = src.as_ptr() as *const T;
    unsafe { Ok(ptr::read_unaligned(src_ptr)) }
}
