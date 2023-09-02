//! The async operations.
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::driver::Driver`], and poll the driver.

use crate::{
    buf::{
        AsIoSlices, AsIoSlicesMut, BufWrapper, IntoInner, IoBuf, IoBufMut, OneOrVec,
        VectoredBufWrapper, WrapBuf,
    },
    driver::{sockaddr_storage, socklen_t, RawFd},
    BufResult,
};
use socket2::SockAddr;
use std::io::{IoSlice, IoSliceMut};

pub(crate) trait BufResultExt {
    fn map_advanced(self) -> Self;
}

impl<T: AsIoSlicesMut> BufResultExt for BufResult<usize, T> {
    fn map_advanced(self) -> Self {
        let (res, buffer) = self;
        let (res, buffer) = (res.map(|res| (res, ())), buffer).map_advanced();
        let res = res.map(|(res, _)| res);
        (res, buffer)
    }
}

impl<T: AsIoSlicesMut, O> BufResultExt for BufResult<(usize, O), T> {
    fn map_advanced(self) -> Self {
        let (res, mut buffer) = self;
        if let Ok((init, _)) = &res {
            buffer.set_init(*init);
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

pub use crate::driver::op::{Accept, RecvFromImpl, SendToImpl};

#[cfg(target_os = "windows")]
pub use crate::driver::op::ConnectNamedPipe;

/// Read a file at specified position into specified buffer.
#[derive(Debug)]
pub struct ReadAt<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: BufWrapper<T>,
}

impl<T: IoBufMut> ReadAt<T> {
    /// Create [`ReadAt`].
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer: BufWrapper::new(buffer),
        }
    }
}

impl<T: IoBufMut> IntoInner for ReadAt<T> {
    type Inner = BufWrapper<T>;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from specified buffer.
#[derive(Debug)]
pub struct WriteAt<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: BufWrapper<T>,
}

impl<T: IoBuf> WriteAt<T> {
    /// Create [`WriteAt`].
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer: BufWrapper::new(buffer),
        }
    }
}

impl<T: IoBuf> IntoInner for WriteAt<T> {
    type Inner = BufWrapper<T>;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

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
pub struct RecvImpl<T: AsIoSlicesMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: OneOrVec<IoSliceMut<'static>>,
}

impl<T: AsIoSlicesMut> RecvImpl<T> {
    /// Create [`Recv`].
    pub fn new(fd: RawFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            slices: OneOrVec::One(IoSliceMut::new(&mut [])),
        }
    }
}

impl<T: AsIoSlicesMut> IntoInner for RecvImpl<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data with one buffer.
pub type Recv<T> = RecvImpl<BufWrapper<T>>;
/// Receive data with vectored buffer.
pub type RecvVectored<T> = RecvImpl<VectoredBufWrapper<T>>;

/// Send data to remote.
pub struct SendImpl<T: AsIoSlices> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: OneOrVec<IoSlice<'static>>,
}

impl<T: AsIoSlices> SendImpl<T> {
    /// Create [`Send`].
    pub fn new(fd: RawFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            slices: OneOrVec::One(IoSlice::new(&[])),
        }
    }
}

impl<T: AsIoSlices> IntoInner for SendImpl<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data with one buffer.
pub type Send<T> = SendImpl<BufWrapper<T>>;
/// Send data with vectored buffer.
pub type SendVectored<T> = SendImpl<VectoredBufWrapper<T>>;

/// Receive data and address with one buffer.
pub type RecvFrom<T> = RecvFromImpl<BufWrapper<T>>;
/// Receive data and address with vectored buffer.
pub type RecvFromVectored<T> = RecvFromImpl<VectoredBufWrapper<T>>;

/// Send data to address with one buffer.
pub type SendTo<T> = SendToImpl<BufWrapper<T>>;
/// Send data to address with vectored buffer.
pub type SendToVectored<T> = SendToImpl<VectoredBufWrapper<T>>;
