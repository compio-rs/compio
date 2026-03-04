//! Ancillary data (control message) support for connected streams.
//!
//! Ancillary messages are used to pass out-of-band information such as file
//! descriptors (Unix domain sockets), credentials, or kTLS record types.
//!
//! # Types
//!
//! - [`AncillaryRef`]: A reference to a single ancillary data entry.
//! - [`AncillaryIter`]: An iterator over a buffer of ancillary messages.
//! - [`AncillaryBuilder`]: A builder for constructing ancillary messages into a
//!   caller-supplied send buffer.
//! - [`AncillaryBuf`]: A fixed-size, properly aligned stack buffer for
//!   ancillary data
//!
//! # Example
//!
//! ```
//! use compio_io::ancillary::{AncillaryBuf, AncillaryIter, ancillary_space};
//!
//! const LEVEL: i32 = 1;
//! const TYPE: i32 = 2;
//!
//! // Build a buffer containing two `u32` ancillary messages.
//! let mut builder = AncillaryBuf::<{ ancillary_space::<u32>() * 2 }>::builder();
//! builder.try_push(LEVEL, TYPE, 42u32).unwrap();
//! builder.try_push(LEVEL, TYPE, 43u32).unwrap();
//! assert!(builder.try_push(LEVEL, TYPE, 44u32).is_none()); // buffer is full
//! let buf = builder.finish();
//!
//! // Read it back.
//! unsafe {
//!     let mut iter = AncillaryIter::new(&buf);
//!     let msg = iter.next().unwrap();
//!     assert_eq!(msg.level(), LEVEL);
//!     assert_eq!(msg.ty(), TYPE);
//!     assert_eq!(*msg.data::<u32>(), 42u32);
//!     assert_eq!(iter.next().unwrap().data::<u32>(), &43u32);
//!     assert!(iter.next().is_none());
//! }
//! ```

use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

use compio_buf::{IoBuf, IoBufMut, SetLen};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}

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
pub struct AncillaryBuilder<const N: usize> {
    buffer: Box<AncillaryBuf<N>>,
    inner: sys::CMsgIter,
}

impl<const N: usize> AncillaryBuilder<N> {
    fn new() -> Self {
        let mut buffer = Box::new(AncillaryBuf::new());
        let inner = sys::CMsgIter::new(buffer.as_uninit().as_ptr().cast(), buffer.buf_capacity());
        Self { buffer, inner }
    }

    /// Finishes building, returns the ancillary buffer containing the
    /// constructed control messages.
    pub fn finish(self) -> AncillaryBuf<N> {
        *self.buffer
    }

    /// Try to append a control message entry into the buffer. If the buffer
    /// does not have enough space or is not properly aligned with the value
    /// type, returns `None`.
    pub fn try_push<T>(&mut self, level: i32, ty: i32, value: T) -> Option<()> {
        if !self.inner.is_aligned::<T>() || !self.inner.is_space_enough::<T>() {
            return None;
        }

        // SAFETY: the buffer is zeroed and the pointer is valid and aligned
        unsafe {
            let mut cmsg = self.inner.current_mut()?;
            cmsg.set_level(level);
            cmsg.set_ty(ty);
            self.buffer.len += cmsg.set_data(value);

            self.inner.next();
        }

        Some(())
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

    /// Creates an [`AncillaryBuilder`] for constructing ancillary messages into
    /// this buffer.
    ///
    /// # Panics
    ///
    /// This function will panic if the buffer size `N` is too small to hold at
    /// least one control message header.
    pub fn builder() -> AncillaryBuilder<N> {
        AncillaryBuilder::new()
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
        if !self.inner.is_aligned::<T>() || !self.inner.is_space_enough::<T>() {
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
/// This is the platform-appropriate equivalent of `CMSG_SPACE(sizeof(T))` on
/// Unix or `WSA_CMSG_SPACE(sizeof(T))` on Windows, and can be used as a const
/// generic argument for [`AncillaryBuf`].
pub const fn ancillary_space<T>() -> usize {
    #[cfg(unix)]
    // SAFETY: CMSG_SPACE is always safe
    unsafe {
        libc::CMSG_SPACE(size_of::<T>() as libc::c_uint) as usize
    }

    #[cfg(windows)]
    sys::wsa_cmsg_space(size_of::<T>())
}
