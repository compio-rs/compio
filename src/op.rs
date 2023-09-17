//! The async operations.
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::driver::Driver`], and poll the driver.

use std::marker::PhantomData;

use socket2::SockAddr;

#[cfg(target_os = "windows")]
pub use crate::driver::op::ConnectNamedPipe;
pub use crate::driver::op::{Accept, RecvFromImpl, RecvImpl, SendImpl, SendToImpl};
use crate::{
    buf::{AsIoSlicesMut, IntoInner, IoBuf, IoBufMut, VectoredBufWrapper},
    driver::{sockaddr_storage, socklen_t, RawFd},
    BufResult,
};

pub(crate) trait BufResultExt {
    fn map_advanced(self) -> Self;
}

impl<'arena, T: AsIoSlicesMut> BufResultExt for BufResult<'arena, usize, T> {
    fn map_advanced(self) -> Self {
        let (res, buffer) = self;
        let (res, buffer) = (res.map(|res| (res, ())), buffer).map_advanced();
        let res = res.map(|(res, _)| res);
        (res, buffer)
    }
}

impl<'arena, T: AsIoSlicesMut, O> BufResultExt for BufResult<'arena, (usize, O), T> {
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

impl<'arena, T: 'arena> RecvResultExt
    for BufResult<'arena, usize, (T, sockaddr_storage, socklen_t)>
{
    type RecvFromResult = BufResult<'arena, (usize, SockAddr), T>;

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
pub struct ReadAt<'arena, T: IoBufMut<'arena>> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: T,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena, T: IoBufMut<'arena>> ReadAt<'arena, T> {
    /// Create [`ReadAt`].
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            _lifetime: PhantomData,
        }
    }
}

impl<'arena, T: IoBufMut<'arena>> IntoInner for ReadAt<'arena, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from specified buffer.
#[derive(Debug)]
pub struct WriteAt<'arena, T: IoBuf<'arena>> {
    pub(crate) fd: RawFd,
    pub(crate) offset: usize,
    pub(crate) buffer: T,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena, T: IoBuf<'arena>> WriteAt<'arena, T> {
    /// Create [`WriteAt`].
    pub fn new(fd: RawFd, offset: usize, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            _lifetime: PhantomData,
        }
    }
}

impl<'arena, T: IoBuf<'arena>> IntoInner for WriteAt<'arena, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Sync data to the disk.
pub struct Sync {
    pub(crate) fd: RawFd,
    #[allow(dead_code)]
    pub(crate) datasync: bool,
}

impl Sync {
    /// Create [`Sync`].
    ///
    /// If `datasync` is `true`, the file metadata may not be synchronized.
    ///
    /// ## Platform specific
    ///
    /// * IOCP: it is synchronized operation, and calls `FlushFileBuffers`.
    /// * io-uring: `fdatasync` if `datasync` specified, otherwise `fsync`.
    /// * mio: it is synchronized `fdatasync` or `fsync`.
    pub fn new(fd: RawFd, datasync: bool) -> Self {
        Self { fd, datasync }
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

/// Receive data with one buffer.
pub type Recv<T> = RecvImpl<T>;
/// Receive data with vectored buffer.
pub type RecvVectored<'arena, T> = RecvImpl<VectoredBufWrapper<'arena, T>>;

/// Send data with one buffer.
pub type Send<T> = SendImpl<T>;
/// Send data with vectored buffer.
pub type SendVectored<'arena, T> = SendImpl<VectoredBufWrapper<'arena, T>>;

/// Receive data and address with one buffer.
pub type RecvFrom<T> = RecvFromImpl<T>;
/// Receive data and address with vectored buffer.
pub type RecvFromVectored<'arena, T> = RecvFromImpl<VectoredBufWrapper<'arena, T>>;

/// Send data to address with one buffer.
pub type SendTo<T> = SendToImpl<T>;
/// Send data to address with vectored buffer.
pub type SendToVectored<'arena, T> = SendToImpl<VectoredBufWrapper<'arena, T>>;
