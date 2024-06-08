#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket, RawSocket};
use std::{io, ops::Deref};

use compio_buf::{BufResult, IntoInner};
#[cfg(unix)]
use compio_driver::op::{Interest, PollOnce};
use compio_driver::{AsRawFd, RawFd, SharedFd, ToSharedFd};

/// A wrapper for socket, providing functionalities to wait for readiness.
#[derive(Debug)]
pub struct PollFd<T: AsRawFd> {
    inner: SharedFd<T>,
}

impl<T: AsRawFd> PollFd<T> {
    /// Create [`PollFd`] without attaching the source. Ready-based sources need
    /// not to be attached.
    pub fn new(source: T) -> Self {
        Self {
            inner: SharedFd::new(source),
        }
    }

    pub(crate) fn from_shared_fd(inner: SharedFd<T>) -> Self {
        Self { inner }
    }
}

#[cfg(unix)]
impl<T: AsRawFd + 'static> PollFd<T> {
    /// Wait for accept readiness, before calling `accept`, or after `accept`
    /// returns `WouldBlock`.
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.read_ready().await
    }

    /// Wait for connect readiness.
    pub async fn connect_ready(&self) -> io::Result<()> {
        self.write_ready().await
    }

    /// Wait for read readiness.
    pub async fn read_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Readable);
        let BufResult(res, _) = compio_runtime::submit(op).await;
        res?;
        Ok(())
    }

    /// Wait for write readiness.
    pub async fn write_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Writable);
        let BufResult(res, _) = compio_runtime::submit(op).await;
        res?;
        Ok(())
    }
}

impl<T: AsRawFd> IntoInner for PollFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.inner
    }
}

impl<T: AsRawFd> ToSharedFd<T> for PollFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.inner.clone()
    }
}

impl<T: AsRawFd> Clone for PollFd<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: AsRawFd> AsRawFd for PollFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: AsRawFd + AsRawSocket> AsRawSocket for PollFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.inner.as_raw_socket()
    }
}

#[cfg(unix)]
impl<T: AsRawFd + FromRawFd> FromRawFd for PollFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new(FromRawFd::from_raw_fd(fd))
    }
}

#[cfg(windows)]
impl<T: AsRawFd + FromRawSocket> FromRawSocket for PollFd<T> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        Self::new(FromRawSocket::from_raw_socket(sock))
    }
}

impl<T: AsRawFd> Deref for PollFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
