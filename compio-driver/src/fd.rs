#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, FromRawSocket, RawHandle, RawSocket};
use std::{
    cell::{Cell, RefCell},
    future::{Future, poll_fn},
    mem::ManuallyDrop,
    ops::Deref,
    panic::RefUnwindSafe,
    rc::Rc,
    task::{Poll, Waker},
};

use crate::{AsFd, AsRawFd, BorrowedFd, RawFd};

#[derive(Debug)]
struct Inner<T> {
    fd: T,
    // whether there is a future waiting
    waits: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}

impl<T> RefUnwindSafe for Inner<T> {}

/// A shared fd. It is passed to the operations to make sure the fd won't be
/// closed before the operations complete.
#[derive(Debug)]
pub struct SharedFd<T>(Rc<Inner<T>>);
// We use `Rc` internally to avoid the overhead of `Arc`. It is not `Send` or
// `Sync`, but we have to access it in another thread when processing blocking
// operations. We ensure that the access is safe because there will be only
// one thread accessing it at a time.

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
        Self(Rc::new(Inner {
            fd,
            waits: Cell::new(false),
            waker: RefCell::new(None),
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
        let ptr = ManuallyDrop::new(std::ptr::read(&this.0));
        // The ptr is duplicated without increasing the strong count, should forget.
        match Rc::try_unwrap(ManuallyDrop::into_inner(ptr)) {
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
            if !this.0.waits.replace(true) {
                poll_fn(move |cx| {
                    if let Some(fd) = unsafe { Self::try_unwrap_inner(&this) } {
                        return Poll::Ready(Some(fd));
                    }

                    this.0.waker.borrow_mut().replace(cx.waker().clone());

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
        if Rc::strong_count(&self.0) == 2 && self.0.waits.get() {
            if let Some(waker) = self.0.waker.borrow_mut().take() {
                waker.wake()
            }
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
        Self::new_unchecked(T::from_raw_handle(handle))
    }
}

#[cfg(windows)]
impl<T: FromRawSocket> FromRawSocket for SharedFd<T> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        Self::new_unchecked(T::from_raw_socket(sock))
    }
}

#[cfg(unix)]
impl<T: FromRawFd> FromRawFd for SharedFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new_unchecked(T::from_raw_fd(fd))
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
