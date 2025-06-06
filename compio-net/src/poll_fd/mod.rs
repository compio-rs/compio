cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}

#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::{io, ops::Deref};

use compio_buf::IntoInner;
use compio_driver::{AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd};

/// A wrapper for socket, providing functionalities to wait for readiness.
#[derive(Debug)]
pub struct PollFd<T: AsFd>(sys::PollFd<T>);

impl<T: AsFd> PollFd<T> {
    /// Create [`PollFd`] without attaching the source. Ready-based sources need
    /// not to be attached.
    pub fn new(source: T) -> io::Result<Self> {
        Self::from_shared_fd(SharedFd::new(source))
    }

    pub(crate) fn from_shared_fd(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self(sys::PollFd::new(inner)?))
    }
}

impl<T: AsFd + AsRawFd + 'static> PollFd<T> {
    /// Wait for accept readiness, before calling `accept`, or after `accept`
    /// returns `WouldBlock`.
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.0.accept_ready().await
    }

    /// Wait for connect readiness.
    pub async fn connect_ready(&self) -> io::Result<()> {
        self.0.connect_ready().await
    }

    /// Wait for read readiness.
    pub async fn read_ready(&self) -> io::Result<()> {
        self.0.read_ready().await
    }

    /// Wait for write readiness.
    pub async fn write_ready(&self) -> io::Result<()> {
        self.0.write_ready().await
    }
}

impl<T: AsFd> IntoInner for PollFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}

impl<T: AsFd> ToSharedFd<T> for PollFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.0.to_shared_fd()
    }
}

impl<T: AsFd> AsFd for PollFd<T> {
    fn as_fd(&self) -> BorrowedFd {
        self.0.as_fd()
    }
}

impl<T: AsFd> AsRawFd for PollFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: AsFd + AsRawSocket> AsRawSocket for PollFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.0.as_raw_socket()
    }
}

impl<T: AsFd> Deref for PollFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
