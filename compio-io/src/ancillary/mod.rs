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
//!
//! # Example
//!
//! Send and receive a file descriptor over a Unix socket pair using
//! `SCM_RIGHTS`:
//!
//! ```
//! # #[cfg(unix)] {
//! use std::os::unix::io::RawFd;
//!
//! use compio_io::ancillary::*;
//! use compio_net::UnixStream;
//!
//! const BUF_SIZE: usize = ancillary_space::<RawFd>();
//!
//! # compio_runtime::Runtime::new().unwrap().block_on(async {
//! // Create a socket pair.
//! let (std_a, std_b) = std::os::unix::net::UnixStream::pair().unwrap();
//! let mut a = UnixStream::from_std(std_a).unwrap();
//! let mut b = UnixStream::from_std(std_b).unwrap();
//!
//! // Pass fd 0 (stdin) as ancillary data via SCM_RIGHTS.
//! let mut ctrl_send = AncillaryBuf::<BUF_SIZE>::new();
//! let mut builder = ctrl_send.builder();
//! builder
//!     .push(libc::SOL_SOCKET, libc::SCM_RIGHTS, &(0 as RawFd))
//!     .unwrap();
//!
//! // Send the payload together with the ancillary data.
//! a.write_with_ancillary(b"hello", ctrl_send).await.0.unwrap();
//!
//! // Receive on the other end.
//! let payload = Vec::with_capacity(5);
//! let ctrl_recv = AncillaryBuf::<BUF_SIZE>::new();
//! let ((_, ctrl_len), (payload, ctrl_recv)) =
//!     b.read_with_ancillary(payload, ctrl_recv).await.unwrap();
//!
//! assert_eq!(&payload[..], b"hello");
//!
//! // Parse the received ancillary messages.
//! let mut iter = unsafe { AncillaryIter::new(&ctrl_recv[..ctrl_len]) };
//! let msg = iter.next().unwrap();
//! assert_eq!(msg.level(), libc::SOL_SOCKET);
//! assert_eq!(msg.ty(), libc::SCM_RIGHTS);
//! // The kernel duplicates the fd, so the received value may differ.
//! let _received_fd = unsafe { msg.data::<RawFd>() };
//! assert!(iter.next().is_none());
//! # });
//! # }
//! ```

use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr,
};

use compio_buf::{IoBuf, IoBufMut, SetLen};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock;

mod io;

pub use self::io::*;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}
#[cfg(feature = "bytemuck")]
pub mod bytemuck_ext;

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
    _p: PhantomData<&'a ()>,
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
            _p: PhantomData,
        }
    }
}

impl<'a> Iterator for AncillaryIter<'a> {
    type Item = AncillaryRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let cmsg = self.inner.current();
            self.inner.next();
            cmsg.map(AncillaryRef)
        }
    }
}

/// Helper to construct ancillary (control) messages.
pub struct AncillaryBuilder<'a, const N: usize> {
    inner: sys::CMsgIter,
    buffer: &'a mut AncillaryBuf<N>,
}

impl<'a, const N: usize> AncillaryBuilder<'a, N> {
    fn new(buffer: &'a mut AncillaryBuf<N>) -> Self {
        // TODO: optimize zeroing
        buffer.as_uninit().fill(MaybeUninit::new(0));
        buffer.len = 0;
        let inner = sys::CMsgIter::new(buffer.as_ptr(), buffer.buf_capacity());
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

        // SAFETY: AncillaryBuf guarantees the buffer is zeroed and properly aligned,
        // and we have checked the space.
        let mut cmsg = unsafe { self.inner.current_mut() }.expect("sufficient space");
        cmsg.set_level(level);
        cmsg.set_ty(ty);
        self.buffer.len += cmsg.encode_data(value)?;

        unsafe { self.inner.next() };

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
    #[cfg(unix)]
    _align: [libc::cmsghdr; 0],
    #[cfg(windows)]
    _align: [WinSock::CMSGHDR; 0],
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
    pub fn builder(&mut self) -> AncillaryBuilder<'_, N> {
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

// Deprecated compio_net::CMsgRef
#[doc(hidden)]
pub struct CMsgRef<'a>(sys::CMsgRef<'a>);

impl CMsgRef<'_> {
    pub fn level(&self) -> i32 {
        self.0.level()
    }

    pub fn ty(&self) -> i32 {
        self.0.ty()
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len() as _
    }

    /// Returns a reference to the data of the control message.
    ///
    /// # Safety
    ///
    /// The data part must be properly aligned and contains an initialized
    /// instance of `T`.
    pub unsafe fn data<T>(&self) -> &T {
        unsafe { self.0.data() }
    }
}

// Deprecated compio_net::CMsgIter
#[doc(hidden)]
pub struct CMsgIter<'a> {
    inner: sys::CMsgIter,
    _p: PhantomData<&'a ()>,
}

impl<'a> CMsgIter<'a> {
    /// Create [`CMsgIter`] with the given buffer.
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
            _p: PhantomData,
        }
    }
}

impl<'a> Iterator for CMsgIter<'a> {
    type Item = CMsgRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let cmsg = self.inner.current();
            self.inner.next();
            cmsg.map(CMsgRef)
        }
    }
}

// Deprecated compio_net::CMsgBuilder
#[doc(hidden)]
pub struct CMsgBuilder<'a> {
    inner: sys::CMsgIter,
    len: usize,
    _p: PhantomData<&'a mut ()>,
}

impl<'a> CMsgBuilder<'a> {
    pub fn new(buffer: &'a mut [MaybeUninit<u8>]) -> Self {
        buffer.fill(MaybeUninit::new(0));
        Self {
            inner: sys::CMsgIter::new(buffer.as_ptr().cast(), buffer.len()),
            len: 0,
            _p: PhantomData,
        }
    }

    pub fn finish(self) -> usize {
        self.len
    }

    pub fn try_push<T>(&mut self, level: i32, ty: i32, value: T) -> Option<()> {
        if !self.inner.is_aligned::<T>() || !self.inner.is_space_enough(std::mem::size_of::<T>()) {
            return None;
        }

        // SAFETY: the buffer is zeroed and the pointer is valid and aligned
        unsafe {
            let mut cmsg = self.inner.current_mut()?;
            cmsg.set_level(level);
            cmsg.set_ty(ty);
            self.len += cmsg.set_data(value);

            self.inner.next();
        }

        Some(())
    }
}

/// Returns the buffer size required to hold one ancillary message carrying a
/// value of type `T`.
///
/// This is the platform-appropriate equivalent of `CMSG_SPACE(T::SIZE)` on
/// Unix or `WSA_CMSG_SPACE(T::SIZE)` on Windows, and can be used as a const
/// generic argument for [`AncillaryBuf`].
pub const fn ancillary_space<T: AncillaryData>() -> usize {
    #[cfg(unix)]
    // SAFETY: CMSG_SPACE is always safe
    unsafe {
        libc::CMSG_SPACE(T::SIZE as libc::c_uint) as usize
    }

    #[cfg(windows)]
    sys::wsa_cmsg_space(T::SIZE)
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
