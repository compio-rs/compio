#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::{
    FromRawHandle, FromRawSocket, OwnedHandle, OwnedSocket, RawHandle, RawSocket,
};
use std::{
    future::{poll_fn, Future},
    mem::ManuallyDrop,
    panic::RefUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll,
};

use futures_util::task::AtomicWaker;

use crate::{AsRawFd, OwnedFd, RawFd};

#[derive(Debug)]
struct Inner {
    fd: OwnedFd,
    // whether there is a future waiting
    waits: AtomicBool,
    waker: AtomicWaker,
}

impl RefUnwindSafe for Inner {}

/// A shared fd. It is passed to the operations to make sure the fd won't be
/// closed before the operations complete.
#[derive(Debug, Clone)]
pub struct SharedFd(Arc<Inner>);

impl SharedFd {
    /// Create the shared fd from an owned fd.
    pub fn new(fd: impl Into<OwnedFd>) -> Self {
        Self(Arc::new(Inner {
            fd: fd.into(),
            waits: AtomicBool::new(false),
            waker: AtomicWaker::new(),
        }))
    }

    /// Try to take the inner owned fd.
    pub fn try_unwrap(self) -> Result<OwnedFd, Self> {
        let this = ManuallyDrop::new(self);
        if let Some(fd) = unsafe { Self::try_unwrap_inner(&this) } {
            Ok(fd)
        } else {
            Err(ManuallyDrop::into_inner(this))
        }
    }

    // SAFETY: if `Some` is returned, the method should not be called again.
    unsafe fn try_unwrap_inner(this: &ManuallyDrop<Self>) -> Option<OwnedFd> {
        let ptr = ManuallyDrop::new(std::ptr::read(&this.0));
        // The ptr is duplicated without increasing the strong count, should forget.
        match Arc::try_unwrap(ManuallyDrop::into_inner(ptr)) {
            Ok(inner) => Some(inner.fd),
            Err(ptr) => {
                std::mem::forget(ptr);
                None
            }
        }
    }

    /// Wait and take the inner owned fd.
    pub fn take(self) -> impl Future<Output = Option<OwnedFd>> {
        let this = ManuallyDrop::new(self);
        async move {
            if !this.0.waits.swap(true, Ordering::AcqRel) {
                poll_fn(move |cx| {
                    if let Some(fd) = unsafe { Self::try_unwrap_inner(&this) } {
                        return Poll::Ready(Some(fd));
                    }

                    this.0.waker.register(cx.waker());

                    if let Some(fd) = unsafe { Self::try_unwrap_inner(&this) } {
                        Poll::Ready(Some(fd))
                    } else {
                        Poll::Pending
                    }
                })
                .await
            } else {
                None
            }
        }
    }
}

impl Drop for SharedFd {
    fn drop(&mut self) {
        // It's OK to wake multiple times.
        if Arc::strong_count(&self.0) == 2 {
            self.0.waker.wake()
        }
    }
}

#[cfg(windows)]
#[doc(hidden)]
impl SharedFd {
    pub unsafe fn to_file(&self) -> ManuallyDrop<std::fs::File> {
        ManuallyDrop::new(std::fs::File::from_raw_handle(self.as_raw_fd() as _))
    }

    pub unsafe fn to_socket(&self) -> ManuallyDrop<socket2::Socket> {
        ManuallyDrop::new(socket2::Socket::from_raw_socket(self.as_raw_fd() as _))
    }
}

#[cfg(unix)]
#[doc(hidden)]
impl SharedFd {
    pub unsafe fn to_file(&self) -> ManuallyDrop<std::fs::File> {
        ManuallyDrop::new(std::fs::File::from_raw_fd(self.as_raw_fd() as _))
    }

    pub unsafe fn to_socket(&self) -> ManuallyDrop<socket2::Socket> {
        ManuallyDrop::new(socket2::Socket::from_raw_fd(self.as_raw_fd() as _))
    }
}

impl AsRawFd for SharedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.fd.as_raw_fd()
    }
}

#[cfg(windows)]
impl FromRawHandle for SharedFd {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        Self::new(OwnedFd::File(OwnedHandle::from_raw_handle(handle)))
    }
}

#[cfg(windows)]
impl FromRawSocket for SharedFd {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        Self::new(OwnedFd::Socket(OwnedSocket::from_raw_socket(sock)))
    }
}

#[cfg(unix)]
impl FromRawFd for SharedFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new(OwnedFd::from_raw_fd(fd))
    }
}

impl From<OwnedFd> for SharedFd {
    fn from(value: OwnedFd) -> Self {
        Self::new(value)
    }
}

/// Get a clone of [`SharedFd`].
pub trait ToSharedFd {
    /// Return a cloned [`SharedFd`].
    fn to_shared_fd(&self) -> SharedFd;
}

impl ToSharedFd for SharedFd {
    fn to_shared_fd(&self) -> SharedFd {
        self.clone()
    }
}
