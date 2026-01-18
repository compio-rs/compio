#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, FromRawSocket, RawHandle, RawSocket};
use std::{
    future::{Future, poll_fn},
    mem::ManuallyDrop,
    ops::Deref,
    panic::RefUnwindSafe,
    sync::atomic::Ordering,
    task::Poll,
};

use crate::{AsFd, AsRawFd, BorrowedFd, RawFd};

cfg_if::cfg_if! {
    if #[cfg(feature = "sync")] {
        use synchrony::sync;
    } else {
        use synchrony::unsync as sync;
    }
}

use sync::{atomic::AtomicBool, shared::Shared, waker_slot::WakerSlot};

#[derive(Debug)]
struct Inner<T> {
    fd: T,
    // whether there is a future waiting
    waits: AtomicBool,
    waker: WakerSlot,
}

impl<T> RefUnwindSafe for Inner<T> {}

/// A shared fd. It is passed to the operations to make sure the fd won't be
/// closed before the operations complete.
#[derive(Debug)]
pub struct SharedFd<T>(Shared<Inner<T>>);

impl<T: AsFd> SharedFd<T> {
    /// Create the shared fd from an owned fd.
    pub fn new(fd: T) -> Self {
        unsafe { Self::new_unchecked(fd) }
    }
}

impl<T> SharedFd<T> {
    /// Create the shared fd.
    ///
    /// # Safety
    /// * T should own the fd.
    pub unsafe fn new_unchecked(fd: T) -> Self {
        Self(Shared::new(Inner {
            fd,
            waits: AtomicBool::new(false),
            waker: WakerSlot::new(),
        }))
    }

    /// Try to take the inner owned fd.
    pub fn try_unwrap(self) -> Result<T, Self> {
        let this = ManuallyDrop::new(self);
        if let Some(fd) = unsafe { Self::try_unwrap_inner(&this) } {
            Ok(fd)
        } else {
            Err(ManuallyDrop::into_inner(this))
        }
    }

    // SAFETY: if `Some` is returned, the method should not be called again.
    unsafe fn try_unwrap_inner(this: &ManuallyDrop<Self>) -> Option<T> {
        // SAFETY: `this` is not dropped here.
        let ptr = unsafe { std::ptr::read(&this.0) };
        // The ptr is duplicated without increasing the strong count, should forget.
        match Shared::try_unwrap(ptr) {
            Ok(inner) => Some(inner.fd),
            Err(ptr) => {
                std::mem::forget(ptr);
                None
            }
        }
    }

    /// Wait and take the inner owned fd.
    pub fn take(self) -> impl Future<Output = Option<T>> {
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

impl<T> Drop for SharedFd<T> {
    fn drop(&mut self) {
        // It's OK to wake multiple times.
        if Shared::strong_count(&self.0) == 2 && self.0.waits.load(Ordering::Acquire) {
            self.0.waker.wake()
        }
    }
}

impl<T: AsFd> AsFd for SharedFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.fd.as_fd()
    }
}

impl<T: AsFd> AsRawFd for SharedFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.as_fd().as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: FromRawHandle> FromRawHandle for SharedFd<T> {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        unsafe { Self::new_unchecked(T::from_raw_handle(handle)) }
    }
}

#[cfg(windows)]
impl<T: FromRawSocket> FromRawSocket for SharedFd<T> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        unsafe { Self::new_unchecked(T::from_raw_socket(sock)) }
    }
}

#[cfg(unix)]
impl<T: FromRawFd> FromRawFd for SharedFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        unsafe { Self::new_unchecked(T::from_raw_fd(fd)) }
    }
}

impl<T> Clone for SharedFd<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Deref for SharedFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0.fd
    }
}

/// Get a clone of [`SharedFd`].
pub trait ToSharedFd<T> {
    /// Return a cloned [`SharedFd`].
    fn to_shared_fd(&self) -> SharedFd<T>;
}

impl<T> ToSharedFd<T> for SharedFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.clone()
    }
}
