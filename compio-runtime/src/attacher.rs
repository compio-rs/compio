#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(windows)]
use std::os::windows::prelude::{OwnedHandle, OwnedSocket};
#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    ops::{Deref, DerefMut},
};

use compio_buf::IntoInner;
use compio_driver::AsRawFd;
#[doc(hidden)]
pub use compio_driver::{FromRawFd, IntoRawFd, RawFd};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::Runtime;

/// Attach a handle to the driver of current thread.
///
/// A handle can and only can attach once to one driver. The attacher will check
/// if it is attached to the current driver.
#[derive(Debug, Clone)]
pub struct Attacher<S> {
    source: S,
    // Make it thread safe.
    once: OnceLock<()>,
}

impl<S: AsRawFd> Attacher<S> {
    /// Create [`Attacher`].
    pub fn new(source: S) -> io::Result<Self> {
        let this = Self {
            source,
            once: OnceLock::new(),
        };
        this.attach()?;
        Ok(this)
    }

    /// Attach the source. This method could be called many times, but if the
    /// action fails, the error will only return once.
    fn attach(&self) -> io::Result<()> {
        let r = Runtime::current();
        let inner = r.inner();
        self.once
            .get_or_try_init(|| inner.attach(self.source.as_raw_fd()))?;
        Ok(())
    }
}

impl<S> Attachable for Attacher<S> {
    fn is_attached(&self) -> bool {
        self.once.get().is_some()
    }
}

impl<S: IntoRawFd> IntoRawFd for Attacher<S> {
    fn into_raw_fd(self) -> RawFd {
        self.source.into_raw_fd()
    }
}

impl<S: FromRawFd> FromRawFd for Attacher<S> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            source: S::from_raw_fd(fd),
            once: OnceLock::from(()),
        }
    }
}

impl<S: TryClone> TryClone for Attacher<S> {
    /// Try clone self with the cloned source. The attach state will be
    /// reserved.
    fn try_clone(&self) -> io::Result<Self> {
        let source = self.source.try_clone()?;
        Ok(Self {
            source,
            once: self.once.clone(),
        })
    }
}

impl<S> IntoInner for Attacher<S> {
    type Inner = S;

    fn into_inner(self) -> Self::Inner {
        self.source
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

/// Represents an attachable resource to driver.
pub trait Attachable {
    /// Check if [`Attachable::attach`] has been called.
    fn is_attached(&self) -> bool;
}

/// Duplicatable file or socket.
pub trait TryClone: Sized {
    /// Duplicate the source.
    fn try_clone(&self) -> io::Result<Self>;
}

impl TryClone for std::fs::File {
    fn try_clone(&self) -> io::Result<Self> {
        std::fs::File::try_clone(self)
    }
}

impl TryClone for socket2::Socket {
    fn try_clone(&self) -> io::Result<Self> {
        socket2::Socket::try_clone(self)
    }
}

#[cfg(windows)]
impl TryClone for OwnedHandle {
    fn try_clone(&self) -> io::Result<Self> {
        OwnedHandle::try_clone(self)
    }
}

#[cfg(windows)]
impl TryClone for OwnedSocket {
    fn try_clone(&self) -> io::Result<Self> {
        OwnedSocket::try_clone(self)
    }
}

#[cfg(unix)]
impl TryClone for OwnedFd {
    fn try_clone(&self) -> io::Result<Self> {
        OwnedFd::try_clone(self)
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! impl_attachable {
    ($t:ty, $inner:ident) => {
        impl $crate::Attachable for $t {
            fn is_attached(&self) -> bool {
                self.$inner.is_attached()
            }
        }
    };
}
