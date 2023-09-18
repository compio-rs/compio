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
    buf::{AsIoSlicesMut, BufWrapperMut, IntoInner, IoBuf, IoBufMut, VectoredBufWrapper},
    driver::{sockaddr_storage, socklen_t, RawFd},
    BufResult,
};

/// Helper trait to update buffer length after kernel updated the buffer
pub trait UpdateBufferLen {
    /// Update length of wrapped buffer
    fn update_buffer_len(self) -> Self;
}

macro_rules! impl_update_buffer_len {
    ($t:ident) => {
        impl<'arena, T: IoBufMut<'arena>> UpdateBufferLen
            for BufResult<'arena, usize, $t<'arena, T>>
        {
            fn update_buffer_len(self) -> Self {
                let (res, mut buffer) = self;
                if let Ok(init) = &res {
                    buffer.set_init(*init);
                }
                (res, buffer)
            }
        }

        impl<'arena, T: IoBufMut<'arena>, O> UpdateBufferLen
            for BufResult<'arena, (usize, O), $t<'arena, T>>
        {
            fn update_buffer_len(self) -> Self {
                let (res, mut buffer) = self;
                if let Ok((init, _)) = &res {
                    buffer.set_init(*init);
                }
                (res, buffer)
            }
        }
    };
}

impl_update_buffer_len!(VectoredBufWrapper);
impl_update_buffer_len!(BufWrapperMut);

impl<'arena, T: IoBufMut<'arena>> UpdateBufferLen for BufResult<'arena, usize, T> {
    fn update_buffer_len(self) -> Self {
        let (res, mut buffer) = self;
        if let Ok(init) = &res {
            buffer.set_buf_init(*init);
        }
        (res, buffer)
    }
}

impl<'arena, T: IoBufMut<'arena>, O> UpdateBufferLen for BufResult<'arena, (usize, O), T> {
    fn update_buffer_len(self) -> Self {
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
pub type Recv<'arena, T> = RecvImpl<'arena, T>;
/// Receive data with vectored buffer.
pub type RecvVectored<'arena, T> = RecvImpl<'arena, VectoredBufWrapper<'arena, T>>;

/// Send data with one buffer.
pub type Send<'arena, T> = SendImpl<'arena, T>;
/// Send data with vectored buffer.
pub type SendVectored<'arena, T> = SendImpl<'arena, VectoredBufWrapper<'arena, T>>;

/// Receive data and address with one buffer.
pub type RecvFrom<'arena, T> = RecvFromImpl<'arena, T>;
/// Receive data and address with vectored buffer.
pub type RecvFromVectored<'arena, T> = RecvFromImpl<'arena, VectoredBufWrapper<'arena, T>>;

/// Send data to address with one buffer.
pub type SendTo<'arena, T> = SendToImpl<'arena, T>;
/// Send data to address with vectored buffer.
pub type SendToVectored<'arena, T> = SendToImpl<'arena, VectoredBufWrapper<'arena, T>>;
