//! The async operations.
//!
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::Proactor`], and poll the driver.

use std::{io, marker::PhantomPinned, mem::ManuallyDrop, net::Shutdown};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, SetLen};
use pin_project_lite::pin_project;
use socket2::{SockAddr, SockAddrStorage, socklen_t};

pub use crate::sys::op::{
    Accept, Recv, RecvFrom, RecvFromVectored, RecvMsg, RecvVectored, Send, SendMsg, SendTo,
    SendToVectored, SendVectored,
};
#[cfg(windows)]
pub use crate::sys::op::{ConnectNamedPipe, DeviceIoControl};
#[cfg(unix)]
pub use crate::sys::op::{
    CreateDir, CreateSocket, FileStat, HardLink, Interest, OpenFile, PathStat, PollOnce,
    ReadVectored, ReadVectoredAt, Rename, Symlink, Unlink, WriteVectored, WriteVectoredAt,
};
#[cfg(io_uring)]
pub use crate::sys::op::{ReadManaged, ReadManagedAt, RecvManaged};
use crate::{Extra, OwnedFd, SharedFd, TakeBuffer, sys::aio::*};

/// Trait to update the buffer length inside the [`BufResult`].
pub trait BufResultExt {
    /// Call [`SetLen::advance_to`] if the result is [`Ok`].
    ///
    /// # Safety
    ///
    /// The result value must be a valid length to advance to.
    unsafe fn map_advanced(self) -> Self;
}

/// Trait to update the buffer length inside the [`BufResult`].
pub trait VecBufResultExt {
    /// Call [`SetLen::advance_vec_to`] if the result is [`Ok`].
    ///
    /// # Safety
    ///
    /// The result value must be a valid length to advance to.
    unsafe fn map_vec_advanced(self) -> Self;
}

impl<T: SetLen + IoBuf> BufResultExt for BufResult<usize, T> {
    unsafe fn map_advanced(self) -> Self {
        unsafe {
            self.map_res(|res| (res, ()))
                .map_advanced()
                .map_res(|(res, _)| res)
        }
    }
}

impl<T: SetLen + IoVectoredBuf> VecBufResultExt for BufResult<usize, T> {
    unsafe fn map_vec_advanced(self) -> Self {
        unsafe {
            self.map_res(|res| (res, ()))
                .map_vec_advanced()
                .map_res(|(res, _)| res)
        }
    }
}

impl<T: SetLen + IoBuf, O> BufResultExt for BufResult<(usize, O), T> {
    unsafe fn map_advanced(self) -> Self {
        self.map(|(init, obj), mut buffer| {
            unsafe {
                buffer.advance_to(init);
            }
            ((init, obj), buffer)
        })
    }
}

impl<T: SetLen + IoVectoredBuf, O> VecBufResultExt for BufResult<(usize, O), T> {
    unsafe fn map_vec_advanced(self) -> Self {
        self.map(|(init, obj), mut buffer| {
            unsafe {
                buffer.advance_vec_to(init);
            }
            ((init, obj), buffer)
        })
    }
}

impl<T: SetLen + IoBuf, C: SetLen + IoBuf, O> BufResultExt
    for BufResult<(usize, usize, O), (T, C)>
{
    unsafe fn map_advanced(self) -> Self {
        self.map(
            |(init_buffer, init_control, obj), (mut buffer, mut control)| {
                unsafe {
                    buffer.advance_to(init_buffer);
                    control.advance_to(init_control);
                }
                ((init_buffer, init_control, obj), (buffer, control))
            },
        )
    }
}

impl<T: SetLen + IoVectoredBuf, C: SetLen + IoBuf, O> VecBufResultExt
    for BufResult<(usize, usize, O), (T, C)>
{
    unsafe fn map_vec_advanced(self) -> Self {
        self.map(
            |(init_buffer, init_control, obj), (mut buffer, mut control)| {
                unsafe {
                    buffer.advance_vec_to(init_buffer);
                    control.advance_to(init_control);
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

impl<T> RecvResultExt for BufResult<usize, (T, SockAddrStorage, socklen_t)> {
    type RecvResult = BufResult<(usize, SockAddr), T>;

    fn map_addr(self) -> Self::RecvResult {
        self.map_buffer(|(buffer, addr_buffer, addr_size)| (buffer, addr_buffer, addr_size, 0))
            .map_addr()
            .map_res(|(res, _, addr)| (res, addr))
    }
}

impl<T> RecvResultExt for BufResult<usize, (T, SockAddrStorage, socklen_t, usize)> {
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

/// Helper trait for [`ReadManagedAt`] and [`RecvManaged`].
pub trait ResultTakeBuffer {
    /// The buffer pool of the op.
    type BufferPool;
    /// The buffer type of the op.
    type Buffer<'a>;

    /// Take the buffer from result.
    fn take_buffer(self, pool: &Self::BufferPool) -> io::Result<Self::Buffer<'_>>;
}

impl<T: TakeBuffer> ResultTakeBuffer for (BufResult<usize, T>, Extra) {
    type Buffer<'a> = T::Buffer<'a>;
    type BufferPool = T::BufferPool;

    fn take_buffer(self, pool: &Self::BufferPool) -> io::Result<Self::Buffer<'_>> {
        let (BufResult(result, op), extra) = self;
        op.take_buffer(pool, result, extra.buffer_id()?)
    }
}

pin_project! {
    /// Spawn a blocking function in the thread pool.
    pub struct Asyncify<F, D> {
        pub(crate) f: Option<F>,
        pub(crate) data: Option<D>,
        _p: PhantomPinned,
    }
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

pin_project! {
    /// Spawn a blocking function with a file descriptor in the thread pool.
    pub struct AsyncifyFd<S, F, D> {
        pub(crate) fd: SharedFd<S>,
        pub(crate) f: Option<F>,
        pub(crate) data: Option<D>,
        _p: PhantomPinned,
    }
}

impl<S, F, D> AsyncifyFd<S, F, D> {
    /// Create [`AsyncifyFd`].
    pub fn new(fd: SharedFd<S>, f: F) -> Self {
        Self {
            fd,
            f: Some(f),
            data: None,
            _p: PhantomPinned,
        }
    }
}

impl<S, F, D> IntoInner for AsyncifyFd<S, F, D> {
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

pin_project! {
    /// Read a file at specified position into specified buffer.
    #[derive(Debug)]
    pub struct ReadAt<T: IoBufMut, S> {
        pub(crate) fd: S,
        pub(crate) offset: u64,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) aiocb: aiocb,
        _p: PhantomPinned,
    }
}

impl<T: IoBufMut, S> ReadAt<T, S> {
    /// Create [`ReadAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            aiocb: new_aiocb(),
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

pin_project! {
    /// Write a file at specified position from specified buffer.
    #[derive(Debug)]
    pub struct WriteAt<T: IoBuf, S> {
        pub(crate) fd: S,
        pub(crate) offset: u64,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) aiocb: aiocb,
        _p: PhantomPinned,
    }
}

impl<T: IoBuf, S> WriteAt<T, S> {
    /// Create [`WriteAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            aiocb: new_aiocb(),
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

pin_project! {
    /// Read a file.
    pub struct Read<T: IoBufMut, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        _p: PhantomPinned,
    }
}

impl<T: IoBufMut, S> Read<T, S> {
    /// Create [`Read`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
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
    _p: PhantomPinned,
}

impl<T: IoBuf, S> Write<T, S> {
    /// Create [`Write`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf, S> IntoInner for Write<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pin_project! {
    /// Sync data to the disk.
    pub struct Sync<S> {
        pub(crate) fd: S,
        pub(crate) datasync: bool,
        pub(crate) aiocb: aiocb,
    }
}

impl<S> Sync<S> {
    /// Create [`Sync`].
    ///
    /// If `datasync` is `true`, the file metadata may not be synchronized.
    pub fn new(fd: S, datasync: bool) -> Self {
        Self {
            fd,
            datasync,
            aiocb: new_aiocb(),
        }
    }
}

/// Shutdown a socket.
pub struct ShutdownSocket<S> {
    pub(crate) fd: S,
    pub(crate) how: Shutdown,
}

impl<S> ShutdownSocket<S> {
    /// Create [`ShutdownSocket`].
    pub fn new(fd: S, how: Shutdown) -> Self {
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
    pub(crate) fd: S,
    pub(crate) addr: SockAddr,
}

impl<S> Connect<S> {
    /// Create [`Connect`]. `fd` should be bound.
    pub fn new(fd: S, addr: SockAddr) -> Self {
        Self { fd, addr }
    }
}

#[cfg(any(not(io_uring), fusion))]
pub(crate) mod managed {
    use std::io;

    use compio_buf::IntoInner;
    use pin_project_lite::pin_project;

    use super::{Read, ReadAt, Recv};
    use crate::{BorrowedBuffer, BufferPool, OwnedBuffer, TakeBuffer};

    pin_project! {
        /// Read a file at specified position into managed buffer.
        pub struct ReadManagedAt<S> {
            #[pin]
            pub(crate) op: ReadAt<OwnedBuffer, S>,
        }
    }

    impl<S> ReadManagedAt<S> {
        /// Create [`ReadManagedAt`].
        pub fn new(fd: S, offset: u64, pool: &BufferPool, len: usize) -> io::Result<Self> {
            #[cfg(fusion)]
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
            _: u16,
        ) -> io::Result<BorrowedBuffer<'_>> {
            let result = result?;
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_poll();
            let slice = self.op.into_inner();
            // SAFETY: result is valid
            let res = unsafe { buffer_pool.create_proxy(slice, result) };
            #[cfg(fusion)]
            let res = BorrowedBuffer::new_poll(res);
            Ok(res)
        }
    }

    pin_project! {
        /// Read a file into managed buffer.
        pub struct ReadManaged<S> {
            #[pin]
            pub(crate) op: Read<OwnedBuffer, S>,
        }
    }

    impl<S> ReadManaged<S> {
        /// Create [`ReadManaged`].
        pub fn new(fd: S, pool: &BufferPool, len: usize) -> io::Result<Self> {
            #[cfg(fusion)]
            let pool = pool.as_poll();
            Ok(Self {
                op: Read::new(fd, pool.get_buffer(len)?),
            })
        }
    }

    impl<S> TakeBuffer for ReadManaged<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            _: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            let result = result?;
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_poll();
            let slice = self.op.into_inner();
            // SAFETY: result is valid
            let res = unsafe { buffer_pool.create_proxy(slice, result) };
            #[cfg(fusion)]
            let res = BorrowedBuffer::new_poll(res);
            Ok(res)
        }
    }

    pin_project! {
        /// Receive data from remote into managed buffer.
        ///
        /// It is only used for socket operations. If you want to read from a pipe,
        /// use [`ReadManaged`].
        pub struct RecvManaged<S> {
            #[pin]
            pub(crate) op: Recv<OwnedBuffer, S>,
        }
    }

    impl<S> RecvManaged<S> {
        /// Create [`RecvManaged`].
        pub fn new(fd: S, pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
            #[cfg(fusion)]
            let pool = pool.as_poll();
            Ok(Self {
                op: Recv::new(fd, pool.get_buffer(len)?, flags),
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
            _: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            let result = result?;
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_poll();
            let slice = self.op.into_inner();
            // SAFETY: result is valid
            let res = unsafe { buffer_pool.create_proxy(slice, result) };
            #[cfg(fusion)]
            let res = BorrowedBuffer::new_poll(res);
            Ok(res)
        }
    }
}

#[cfg(not(io_uring))]
pub use managed::*;
