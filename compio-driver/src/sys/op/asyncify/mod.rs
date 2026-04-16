use crate::sys::prelude::*;

#[cfg(io_uring)]
mod iour;

#[cfg(polling)]
mod poll;

#[cfg(windows)]
mod iocp;

#[cfg(stub)]
mod stub;

/// Spawn a blocking function in the thread pool.
pub struct Asyncify<F, D> {
    pub(crate) f: Option<F>,
    pub(crate) data: Option<D>,
}

impl<F, D> Asyncify<F, D> {
    /// Create [`Asyncify`].
    pub fn new(f: F) -> Self {
        Self {
            f: Some(f),
            data: None,
        }
    }
}

impl<F, D> IntoInner for Asyncify<F, D> {
    type Inner = D;

    fn into_inner(mut self) -> Self::Inner {
        self.data.take().expect("the data should not be None")
    }
}

/// Spawn a blocking function with a file descriptor in the thread pool.
pub struct AsyncifyFd<S, F, D> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) f: Option<F>,
    pub(crate) data: Option<D>,
}

impl<S, F, D> AsyncifyFd<S, F, D> {
    /// Create [`AsyncifyFd`].
    pub fn new(fd: SharedFd<S>, f: F) -> Self {
        Self {
            fd,
            f: Some(f),
            data: None,
        }
    }
}

impl<S, F, D> IntoInner for AsyncifyFd<S, F, D> {
    type Inner = D;

    fn into_inner(mut self) -> Self::Inner {
        self.data.take().expect("the data should not be None")
    }
}

/// Spawn a blocking function with two file descriptors in the thread pool.
pub struct AsyncifyFd2<S1, S2, F, D> {
    pub(crate) fd1: SharedFd<S1>,
    pub(crate) fd2: SharedFd<S2>,
    pub(crate) f: Option<F>,
    pub(crate) data: Option<D>,
}

impl<S1, S2, F, D> AsyncifyFd2<S1, S2, F, D> {
    /// Create [`AsyncifyFd2`].
    pub fn new(fd1: SharedFd<S1>, fd2: SharedFd<S2>, f: F) -> Self {
        Self {
            fd1,
            fd2,
            f: Some(f),
            data: None,
        }
    }
}

impl<S1, S2, F, D> IntoInner for AsyncifyFd2<S1, S2, F, D> {
    type Inner = D;

    fn into_inner(mut self) -> Self::Inner {
        self.data.take().expect("the data should not be None")
    }
}
