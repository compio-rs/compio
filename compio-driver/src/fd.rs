use std::{
    future::{poll_fn, ready, Future},
    mem::ManuallyDrop,
    panic::RefUnwindSafe,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::Poll,
};

use futures_util::{future::Either, task::AtomicWaker};

use crate::{AsRawFd, OwnedFd, RawFd};

#[derive(Debug)]
struct Inner {
    fd: OwnedFd,
    waits: AtomicUsize,
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
            waits: AtomicUsize::new(0),
            waker: AtomicWaker::new(),
        }))
    }

    /// Try to take the inner owned fd.
    pub fn try_owned(self) -> Result<OwnedFd, Self> {
        let this = ManuallyDrop::new(self);
        if let Some(fd) = Self::try_owned_inner(&this) {
            Ok(fd)
        } else {
            Err(ManuallyDrop::into_inner(this))
        }
    }

    fn try_owned_inner(this: &ManuallyDrop<Self>) -> Option<OwnedFd> {
        // SAFETY: see ManuallyDrop::take
        let ptr = ManuallyDrop::new(unsafe { std::ptr::read(&this.0) });
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
        if self.0.waits.fetch_add(1, Ordering::AcqRel) == 0 {
            let this = ManuallyDrop::new(self);
            Either::Left(async move {
                poll_fn(|cx| {
                    if let Some(fd) = Self::try_owned_inner(&this) {
                        return Poll::Ready(Some(fd));
                    }

                    this.0.waker.register(cx.waker());

                    if let Some(fd) = Self::try_owned_inner(&this) {
                        Poll::Ready(Some(fd))
                    } else {
                        Poll::Pending
                    }
                })
                .await
            })
        } else {
            Either::Right(ready(None))
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
        use std::os::windows::io::FromRawHandle;

        ManuallyDrop::new(std::fs::File::from_raw_handle(self.as_raw_fd() as _))
    }

    pub unsafe fn to_socket(&self) -> ManuallyDrop<socket2::Socket> {
        use std::os::windows::io::FromRawSocket;

        ManuallyDrop::new(socket2::Socket::from_raw_socket(self.as_raw_fd() as _))
    }
}

impl AsRawFd for SharedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.fd.as_raw_fd()
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