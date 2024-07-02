//! The async operations.
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::Proactor`], and poll the driver.

use std::{marker::PhantomPinned, mem::ManuallyDrop, net::Shutdown};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit};
#[cfg(unix)]
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{socklen_t, SOCKADDR_STORAGE as sockaddr_storage};

#[cfg(windows)]
pub use crate::sys::op::ConnectNamedPipe;
pub use crate::sys::op::{
    Accept, Recv, RecvFrom, RecvFromVectored, RecvVectored, Send, SendTo, SendToVectored,
    SendVectored,
};
#[cfg(unix)]
pub use crate::sys::op::{
    CreateDir, CreateSocket, FileStat, HardLink, Interest, OpenFile, PathStat, PollOnce,
    ReadVectoredAt, Rename, Symlink, Unlink, WriteVectoredAt,
};
use crate::{OwnedFd, SharedFd};

/// Trait to update the buffer length inside the [`BufResult`].
pub trait BufResultExt {
    /// Call [`SetBufInit::set_buf_init`] if the result is [`Ok`].
    fn map_advanced(self) -> Self;
}

impl<T: SetBufInit> BufResultExt for BufResult<usize, T> {
    fn map_advanced(self) -> Self {
        self.map_res(|res| (res, ()))
            .map_advanced()
            .map_res(|(res, _)| res)
    }
}

impl<T: SetBufInit, O> BufResultExt for BufResult<(usize, O), T> {
    fn map_advanced(self) -> Self {
        self.map(|(init, obj), mut buffer| {
            unsafe {
                buffer.set_buf_init(init);
            }
            ((init, obj), buffer)
        })
    }
}

/// Helper trait for [`RecvFrom`] and [`RecvFromVectored`].
pub trait RecvResultExt {
    /// The mapped result.
    type RecvFromResult;

    /// Create [`SockAddr`] if the result is [`Ok`].
    fn map_addr(self) -> Self::RecvFromResult;
}

impl<T> RecvResultExt for BufResult<usize, (T, sockaddr_storage, socklen_t)> {
    type RecvFromResult = BufResult<(usize, SockAddr), T>;

    fn map_addr(self) -> Self::RecvFromResult {
        self.map2(
            |res, (buffer, addr_buffer, addr_size)| {
                let addr = unsafe { SockAddr::new(addr_buffer, addr_size) };
                ((res, addr), buffer)
            },
            |(buffer, ..)| buffer,
        )
    }
}

/// Spawn a blocking function in the thread pool.
pub struct Asyncify<F, D> {
    pub(crate) f: Option<F>,
    pub(crate) data: Option<D>,
    _p: PhantomPinned,
}

impl<F, D> Asyncify<F, D> {
    /// Create [`Asyncify`].
    pub fn new(f: F) -> Self {
        Self {
            f: Some(f),
            data: None,
            _p: PhantomPinned,
        }
    }
}

impl<F, D> IntoInner for Asyncify<F, D> {
    type Inner = D;

    fn into_inner(mut self) -> Self::Inner {
        self.data.take().expect("the data should not be None")
    }
}

/// Close the file fd.
pub struct CloseFile {
    pub(crate) fd: ManuallyDrop<OwnedFd>,
}

impl CloseFile {
    /// Create [`CloseFile`].
    pub fn new(fd: OwnedFd) -> Self {
        Self {
            fd: ManuallyDrop::new(fd),
        }
    }
}

/// Read a file at specified position into specified buffer.
#[derive(Debug)]
pub struct ReadAt<T: IoBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> ReadAt<T, S> {
    /// Create [`ReadAt`].
    pub fn new(fd: SharedFd<S>, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            _p: PhantomPinned,
        }
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
    pub(crate) fd: SharedFd<S>,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> WriteAt<T, S> {
    /// Create [`WriteAt`].
    pub fn new(fd: SharedFd<S>, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf, S> IntoInner for WriteAt<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Sync data to the disk.
pub struct Sync<S> {
    pub(crate) fd: SharedFd<S>,
    #[allow(dead_code)]
    pub(crate) datasync: bool,
}

impl<S> Sync<S> {
    /// Create [`Sync`].
    ///
    /// If `datasync` is `true`, the file metadata may not be synchronized.
    pub fn new(fd: SharedFd<S>, datasync: bool) -> Self {
        Self { fd, datasync }
    }
}

/// Shutdown a socket.
pub struct ShutdownSocket<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) how: Shutdown,
}

impl<S> ShutdownSocket<S> {
    /// Create [`ShutdownSocket`].
    pub fn new(fd: SharedFd<S>, how: Shutdown) -> Self {
        Self { fd, how }
    }
}

/// Close socket fd.
pub struct CloseSocket {
    pub(crate) fd: ManuallyDrop<OwnedFd>,
}

impl CloseSocket {
    /// Create [`CloseSocket`].
    pub fn new(fd: OwnedFd) -> Self {
        Self {
            fd: ManuallyDrop::new(fd),
        }
    }
}

/// Connect to a remote address.
pub struct Connect<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) addr: SockAddr,
}

impl<S> Connect<S> {
    /// Create [`Connect`]. `fd` should be bound.
    pub fn new(fd: SharedFd<S>, addr: SockAddr) -> Self {
        Self { fd, addr }
    }
}
