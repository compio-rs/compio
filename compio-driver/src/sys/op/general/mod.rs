//! Implementation of common op codes

#[cfg(windows)]
mod_use![iocp];

#[cfg(io_uring)]
mod_use![iour];

#[cfg(polling)]
mod_use![poll];

#[cfg(stub)]
mod_use![stub];

use crate::sys::prelude::*;

/// Read a file at specified position into specified buffer.
#[derive(Debug)]
pub struct ReadAt<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
}

impl<T: IoBufMut, S> ReadAt<T, S> {
    /// Create [`ReadAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self { fd, offset, buffer }
    }
}

impl<T: IoBufMut, S> IntoInner for ReadAt<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from specified buffer.
#[derive(Debug)]
pub struct WriteAt<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
}

impl<T: IoBuf, S> WriteAt<T, S> {
    /// Create [`WriteAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self { fd, offset, buffer }
    }
}

impl<T: IoBuf, S> IntoInner for WriteAt<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Read a file.
pub struct Read<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
}

impl<T: IoBufMut, S> Read<T, S> {
    /// Create [`Read`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoBufMut, S> IntoInner for Read<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file.
pub struct Write<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
}

impl<T: IoBuf, S> Write<T, S> {
    /// Create [`Write`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoBuf, S> IntoInner for Write<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
