//! The async operations.
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::driver::Driver`], and poll the driver.

use crate::{
    buf::{IntoInner, IoBuf, IoBufMut},
    driver::{sockaddr_storage, socklen_t, RawFd},
    BufResult,
};
use socket2::SockAddr;

pub(crate) trait BufResultExt {
    fn map_advanced(self) -> Self;
}

impl<T: IoBufMut> BufResultExt for BufResult<usize, T> {
    fn map_advanced(self) -> Self {
        let (res, buffer) = self;
        let (res, buffer) = (res.map(|res| (res, ())), buffer).map_advanced();
        let res = res.map(|(res, _)| res);
        (res, buffer)
    }
}

impl<T: IoBufMut, O> BufResultExt for BufResult<(usize, O), T> {
    fn map_advanced(self) -> Self {
        let (res, mut buffer) = self;
        if let Ok((init, _)) = &res {
            buffer.set_buf_init(*init);
        }
        (res, buffer)
    }
}

pub(crate) trait RecvResultExt {
    type RecvFromResult;

    fn map_addr(self) -> Self::RecvFromResult;
}

impl<T> RecvResultExt for BufResult<usize, (T, sockaddr_storage, socklen_t)> {
    type RecvFromResult = BufResult<(usize, SockAddr), T>;

    fn map_addr(self) -> Self::RecvFromResult {
        let (res, (buffer, addr_buffer, addr_size)) = self;
        let res = res.map(|res| {
            let addr = unsafe { SockAddr::new(addr_buffer, addr_size) };
            (res, addr)
        });
        (res, buffer)
    }
}

/// Read a file at specified position into specified buffer.
#[derive(Debug)]
pub struct ReadAt<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: T,
}

impl<T: IoBufMut> ReadAt<T> {
    /// Create [`ReadAt`].
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self { fd, offset, buffer }
    }
}

impl<T: IoBufMut> IntoInner for ReadAt<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from specified buffer.
#[derive(Debug)]
pub struct WriteAt<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: T,
}

impl<T: IoBuf> WriteAt<T> {
    /// Create [`WriteAt`].
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self { fd, offset, buffer }
    }
}

impl<T: IoBuf> IntoInner for WriteAt<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pub use crate::driver::op::Accept;

/// Connect to a remote address.
pub struct Connect {
    pub(crate) fd: RawFd,
    pub(crate) addr: SockAddr,
}

impl Connect {
    /// Create [`Connect`]. `fd` should be bound.
    pub fn new(fd: RawFd, addr: SockAddr) -> Self {
        Self { fd, addr }
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

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
        }
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self { fd, buffer, addr }
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
