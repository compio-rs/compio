use std::{ffi::CString, net::Shutdown};

use compio_buf::{
    IntoInner, IoBuf, IoBufMut, IoSlice, IoSliceMut, IoVectoredBuf, IoVectoredBufMut,
};
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

use crate::{op::*, sys::RawFd};

/// Open or create a file with flags and mode.
pub struct OpenFile {
    pub(crate) path: CString,
    pub(crate) flags: i32,
    pub(crate) mode: libc::mode_t,
}

impl OpenFile {
    /// Create [`OpenFile`].
    pub fn new(path: CString, flags: i32, mode: libc::mode_t) -> Self {
        Self { path, flags, mode }
    }
}

/// Read a file at specified position into vectored buffer.
pub struct ReadVectoredAt<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
}

impl<T: IoVectoredBufMut> ReadVectoredAt<T> {
    /// Create [`ReadVectoredAt`].
    pub fn new(fd: RawFd, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for ReadVectoredAt<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from vectored buffer.
pub struct WriteVectoredAt<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
}

impl<T: IoVectoredBuf> WriteVectoredAt<T> {
    /// Create [`WriteVectoredAt`]
    pub fn new(fd: RawFd, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
        }
    }
}

impl<T: IoVectoredBuf> IntoInner for WriteVectoredAt<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl ShutdownSocket {
    pub(crate) fn how(&self) -> i32 {
        match self.how {
            Shutdown::Write => libc::SHUT_WR,
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Both => libc::SHUT_RDWR,
        }
    }
}

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: RawFd,
    pub(crate) buffer: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
}

impl Accept {
    /// Create [`Accept`].
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            buffer: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
        }
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> SockAddr {
        unsafe { SockAddr::new(self.buffer, self.addr_len) }
    }
}

/// Receive data from remote.
pub struct Recv<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
}

impl<T: IoBufMut> Recv<T> {
    /// Create [`Recv`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoBufMut> IntoInner for Recv<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
}

impl<T: IoVectoredBufMut> RecvVectored<T> {
    /// Create [`RecvVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
}

impl<T: IoBuf> Send<T> {
    /// Create [`Send`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoBuf> IntoInner for Send<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
}

impl<T: IoVectoredBuf> SendVectored<T> {
    /// Create [`SendVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
        }
    }
}

impl<T: IoVectoredBuf> IntoInner for SendVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
