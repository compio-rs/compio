use std::io;
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(windows)]
use std::os::windows::prelude::{OwnedHandle, OwnedSocket};
#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;

use compio_buf::IntoInner;
use compio_driver::AsRawFd;
#[doc(hidden)]
pub use compio_driver::{FromRawFd, IntoRawFd, RawFd};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::Runtime;

/// Attach a handle to the driver of current thread.
///
/// A handle can and only can attach once to one driver. However, the handle
/// itself is Send & Sync. We mark it !Send & !Sync to warn users, making them
/// ensure that they are using it in the correct thread.
#[derive(Debug, Clone)]
pub struct Attacher<S> {
    source: S,
    // Make it thread safe.
    once: OnceLock<usize>,
}

impl<S> Attacher<S> {
    /// Create [`Attacher`].
    pub fn new(source: S) -> Self {
        Self {
            source,
            once: OnceLock::new(),
        }
    }
}

impl<S: AsRawFd> Attacher<S> {
    /// Attach the source. This method could be called many times, but if the
    /// action fails, the error will only return once.
    fn attach(&self) -> io::Result<()> {
        let r = Runtime::current();
        let inner = r.inner();
        let id = self.once.get_or_try_init(|| {
            inner.attach(self.source.as_raw_fd())?;
            io::Result::Ok(inner.id())
        })?;
        if id != &inner.id() {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the current runtime is not the attached runtime",
            ))
        } else {
            Ok(())
        }
    }

    /// Attach the inner source and get the reference.
    pub fn try_get(&self) -> io::Result<&S> {
        self.attach()?;
        Ok(&self.source)
    }

    /// Attach the inner source and get the mutable reference.
    pub fn try_get_mut(&mut self) -> io::Result<&mut S> {
        self.attach()?;
        Ok(&mut self.source)
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
        Self::new(S::from_raw_fd(fd))
    }
}

impl<S: TryClone + AsRawFd> TryClone for Attacher<S> {
    /// Try clone self with the cloned source. The attach state will be
    /// reserved.
    ///
    /// ## Platform specific
    /// * io-uring/polling: it will try to attach in the current thread if
    ///   needed.
    fn try_clone(&self) -> io::Result<Self> {
        let source = self.source.try_clone()?;
        let new_self = if cfg!(windows) {
            Self {
                source,
                once: self.once.clone(),
            }
        } else {
            let new_self = Self::new(source);
            if self.is_attached() {
                new_self.attach()?;
            }
            new_self
        };
        Ok(new_self)
    }
}

impl<S> IntoInner for Attacher<S> {
    type Inner = S;

    fn into_inner(self) -> Self::Inner {
        self.source
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

/// Extracts raw fds.
pub trait TryAsRawFd {
    /// Get the inner raw fd, while ensuring the source being attached.
    fn try_as_raw_fd(&self) -> io::Result<RawFd>;

    /// Get the inner raw fd and don't check if it has been attached.
    ///
    /// # Safety
    ///
    /// The caller should ensure it is attached before submit an operation with
    /// it.
    unsafe fn as_raw_fd_unchecked(&self) -> RawFd;
}

impl<T: AsRawFd> TryAsRawFd for T {
    fn try_as_raw_fd(&self) -> io::Result<RawFd> {
        Ok(self.as_raw_fd())
    }

    unsafe fn as_raw_fd_unchecked(&self) -> RawFd {
        self.as_raw_fd()
    }
}

impl<S: AsRawFd> TryAsRawFd for Attacher<S> {
    fn try_as_raw_fd(&self) -> io::Result<RawFd> {
        Ok(self.try_get()?.as_raw_fd())
    }

    unsafe fn as_raw_fd_unchecked(&self) -> RawFd {
        self.source.as_raw_fd()
    }
}

/// A [`Send`] wrapper for attachable resource that has not been attached. The
/// resource should be able to send to another thread before attaching.
pub struct Unattached<T: Attachable>(T);

impl<T: Attachable> Unattached<T> {
    /// Create the [`Unattached`] wrapper, or fail if the resource has already
    /// been attached.
    pub fn new(a: T) -> Result<Self, T> {
        if a.is_attached() { Err(a) } else { Ok(Self(a)) }
    }

    /// Create [`Unattached`] without checking.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the resource has not been attached.
    pub unsafe fn new_unchecked(a: T) -> Self {
        Self(a)
    }
}

impl<T: Attachable> IntoInner for Unattached<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.0
    }
}

unsafe impl<T: Attachable> Send for Unattached<T> {}
unsafe impl<T: Attachable> Sync for Unattached<T> {}

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

#[macro_export]
#[doc(hidden)]
macro_rules! impl_try_as_raw_fd {
    ($t:ty, $inner:ident) => {
        impl $crate::TryAsRawFd for $t {
            fn try_as_raw_fd(&self) -> ::std::io::Result<$crate::RawFd> {
                self.$inner.try_as_raw_fd()
            }

            unsafe fn as_raw_fd_unchecked(&self) -> $crate::RawFd {
                self.$inner.as_raw_fd_unchecked()
            }
        }
        impl $crate::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: $crate::RawFd) -> Self {
                Self {
                    $inner: $crate::FromRawFd::from_raw_fd(fd),
                }
            }
        }
        impl $crate::IntoRawFd for $t {
            fn into_raw_fd(self) -> $crate::RawFd {
                self.$inner.into_raw_fd()
            }
        }
    };
}
