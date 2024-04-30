#[cfg(unix)]
use std::os::fd::{FromRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, FromRawSocket, RawHandle, RawSocket};
use std::{
    io,
    ops::{Deref, DerefMut},
};

use compio_buf::IntoInner;
use compio_driver::AsRawFd;

use crate::Runtime;

/// Attach a handle to the driver of current thread.
///
/// A handle can and only can attach once to one driver. The attacher will try
/// to attach the handle.
#[derive(Debug, Clone)]
pub struct Attacher<S> {
    source: S,
}

impl<S> Attacher<S> {
    /// Create [`Attacher`] without trying to attach the source.
    ///
    /// # Safety
    ///
    /// The user should ensure that the source is attached to the current
    /// driver.
    pub unsafe fn new_unchecked(source: S) -> Self {
        Self { source }
    }
}

impl<S: AsRawFd> Attacher<S> {
    /// Create [`Attacher`]. It tries to attach the source, and will return
    /// [`Err`] if it fails.
    ///
    /// ## Platform specific
    /// * IOCP: a handle could not be attached more than once. If you want to
    ///   clone the handle, create the [`Attacher`] before cloning.
    pub fn new(source: S) -> io::Result<Self> {
        let r = Runtime::current();
        let inner = r.inner();
        inner.attach(source.as_raw_fd())?;
        Ok(Self { source })
    }
}

impl<S> IntoInner for Attacher<S> {
    type Inner = S;

    fn into_inner(self) -> Self::Inner {
        self.source
    }
}

#[cfg(windows)]
impl<S: FromRawHandle> FromRawHandle for Attacher<S> {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        Self::new_unchecked(S::from_raw_handle(handle))
    }
}

#[cfg(windows)]
impl<S: FromRawSocket> FromRawSocket for Attacher<S> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        Self::new_unchecked(S::from_raw_socket(sock))
    }
}

#[cfg(unix)]
impl<S: FromRawFd> FromRawFd for Attacher<S> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new_unchecked(S::from_raw_fd(fd))
    }
}

impl<S> Deref for Attacher<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

impl<S> DerefMut for Attacher<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.source
    }
}
