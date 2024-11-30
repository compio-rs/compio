//! The async operations.
//!
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::Proactor`], and poll the driver.

use std::{marker::PhantomPinned, mem::ManuallyDrop, net::Shutdown};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit};
use socket2::SockAddr;

#[cfg(windows)]
pub use crate::sys::op::ConnectNamedPipe;
pub use crate::sys::op::{
    Accept, Recv, RecvFrom, RecvFromVectored, RecvMsg, RecvVectored, Send, SendMsg, SendTo,
    SendToVectored, SendVectored,
};
#[cfg(unix)]
pub use crate::sys::op::{
    CreateDir, CreateSocket, FileStat, HardLink, Interest, OpenFile, PathStat, PollOnce,
    ReadVectoredAt, Rename, Symlink, Unlink, WriteVectoredAt,
};
#[cfg(buf_ring)]
pub use crate::sys::op::{ReadManagedAt, RecvManaged};
use crate::{
    OwnedFd, SharedFd,
    sys::{sockaddr_storage, socklen_t},
};

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

impl<T: SetBufInit, C: SetBufInit, O> BufResultExt for BufResult<(usize, usize, O), (T, C)> {
    fn map_advanced(self) -> Self {
        self.map(
            |(init_buffer, init_control, obj), (mut buffer, mut control)| {
                unsafe {
                    buffer.set_buf_init(init_buffer);
                    control.set_buf_init(init_control);
                }
                ((init_buffer, init_control, obj), (buffer, control))
            },
        )
    }
}

/// Helper trait for [`RecvFrom`], [`RecvFromVectored`] and [`RecvMsg`].
pub trait RecvResultExt {
    /// The mapped result.
    type RecvResult;

    /// Create [`SockAddr`] if the result is [`Ok`].
    fn map_addr(self) -> Self::RecvResult;
}

impl<T> RecvResultExt for BufResult<usize, (T, sockaddr_storage, socklen_t)> {
    type RecvResult = BufResult<(usize, SockAddr), T>;

    fn map_addr(self) -> Self::RecvResult {
        self.map_buffer(|(buffer, addr_buffer, addr_size)| (buffer, addr_buffer, addr_size, 0))
            .map_addr()
            .map_res(|(res, _, addr)| (res, addr))
    }
}

impl<T> RecvResultExt for BufResult<usize, (T, sockaddr_storage, socklen_t, usize)> {
    type RecvResult = BufResult<(usize, usize, SockAddr), T>;

    fn map_addr(self) -> Self::RecvResult {
        self.map2(
            |res, (buffer, addr_buffer, addr_size, len)| {
                let addr = unsafe { SockAddr::new(addr_buffer, addr_size) };
                ((res, len, addr), buffer)
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
    #[cfg(aio)]
    pub(crate) aiocb: libc::aiocb,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> ReadAt<T, S> {
    /// Create [`ReadAt`].
    pub fn new(fd: SharedFd<S>, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            #[cfg(aio)]
            aiocb: unsafe { std::mem::zeroed() },
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
    #[cfg(aio)]
    pub(crate) aiocb: libc::aiocb,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> WriteAt<T, S> {
    /// Create [`WriteAt`].
    pub fn new(fd: SharedFd<S>, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            #[cfg(aio)]
            aiocb: unsafe { std::mem::zeroed() },
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
    #[cfg(aio)]
    pub(crate) aiocb: libc::aiocb,
}

impl<S> Sync<S> {
    /// Create [`Sync`].
    ///
    /// If `datasync` is `true`, the file metadata may not be synchronized.
    pub fn new(fd: SharedFd<S>, datasync: bool) -> Self {
        Self {
            fd,
            datasync,
            #[cfg(aio)]
            aiocb: unsafe { std::mem::zeroed() },
        }
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

#[cfg(any(not(buf_ring), fusion))]
pub(crate) mod managed {
    use std::io;

    use compio_buf::{IntoInner, Slice};

    use super::{ReadAt, Recv};
    use crate::{BorrowedBuffer, BufferPool, SharedFd, TakeBuffer};

    /// Read a file at specified position into managed buffer.
    pub struct ReadManagedAt<S> {
        pub(crate) op: ReadAt<Slice<Vec<u8>>, S>,
    }

    impl<S> ReadManagedAt<S> {
        /// Create [`ReadManagedAt`].
        pub fn new(
            fd: SharedFd<S>,
            offset: u64,
            pool: &BufferPool,
            len: usize,
        ) -> io::Result<Self> {
            #[cfg(all(buf_ring, fusion))]
            let pool = pool.as_poll();
            Ok(Self {
                op: ReadAt::new(fd, offset, pool.get_buffer(len)?),
            })
        }
    }

    impl<S> TakeBuffer for ReadManagedAt<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &BufferPool,
            result: io::Result<usize>,
            _: u32,
        ) -> io::Result<BorrowedBuffer> {
            let result = result?;
            #[cfg(all(buf_ring, fusion))]
            let buffer_pool = buffer_pool.as_poll();
            let slice = self.op.into_inner();
            // Safety: result is valid
            let res = unsafe { buffer_pool.create_proxy(slice, result) };
            #[cfg(all(buf_ring, fusion))]
            let res = BorrowedBuffer::new_poll(res);
            Ok(res)
        }
    }

    /// Receive data from remote into managed buffer.
    pub struct RecvManaged<S> {
        pub(crate) op: Recv<Slice<Vec<u8>>, S>,
    }

    impl<S> RecvManaged<S> {
        /// Create [`RecvManaged`].
        pub fn new(fd: SharedFd<S>, pool: &BufferPool, len: usize) -> io::Result<Self> {
            #[cfg(all(buf_ring, fusion))]
            let pool = pool.as_poll();
            Ok(Self {
                op: Recv::new(fd, pool.get_buffer(len)?),
            })
        }
    }

    impl<S> TakeBuffer for RecvManaged<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            _: u32,
        ) -> io::Result<Self::Buffer<'_>> {
            let result = result?;
            #[cfg(all(buf_ring, fusion))]
            let buffer_pool = buffer_pool.as_poll();
            let slice = self.op.into_inner();
            // Safety: result is valid
            let res = unsafe { buffer_pool.create_proxy(slice, result) };
            #[cfg(all(buf_ring, fusion))]
            let res = BorrowedBuffer::new_poll(res);
            Ok(res)
        }
    }
}

#[cfg(not(buf_ring))]
pub use managed::*;
