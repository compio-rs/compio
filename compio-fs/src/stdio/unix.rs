use std::io;

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::{AsFd, AsRawFd, BorrowedBuffer, BorrowedFd, BufferPool, RawFd};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};

#[cfg(doc)]
use super::{stderr, stdin, stdout};
use crate::AsyncFd;

#[derive(Debug)]
struct StaticFd(RawFd);

impl AsFd for StaticFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.0) }
    }
}

impl AsRawFd for StaticFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0 as _
    }
}

/// A handle to the standard input stream of a process.
///
/// See [`stdin`].
#[derive(Debug, Clone)]
pub struct Stdin(AsyncFd<StaticFd>);

impl Stdin {
    pub(crate) fn new() -> Self {
        // SAFETY: no need to attach on unix
        Self(unsafe { AsyncFd::new_unchecked(StaticFd(libc::STDIN_FILENO)) })
    }
}

impl AsyncRead for Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        (&*self).read_vectored(buf).await
    }
}

impl AsyncRead for &Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&self.0).read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        (&self.0).read_vectored(buf).await
    }
}

impl AsyncReadManaged for Stdin {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        (&*self).read_managed(buffer_pool, len).await
    }
}

impl AsyncReadManaged for &Stdin {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        (&self.0).read_managed(buffer_pool, len).await
    }
}

impl AsRawFd for Stdin {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// A handle to the standard output stream of a process.
///
/// See [`stdout`].
#[derive(Debug, Clone)]
pub struct Stdout(AsyncFd<StaticFd>);

impl Stdout {
    pub(crate) fn new() -> Self {
        // SAFETY: no need to attach on unix
        Self(unsafe { AsyncFd::new_unchecked(StaticFd(libc::STDOUT_FILENO)) })
    }
}

impl AsyncWrite for Stdout {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

impl AsRawFd for Stdout {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// A handle to the standard output stream of a process.
///
/// See [`stderr`].
#[derive(Debug, Clone)]
pub struct Stderr(AsyncFd<StaticFd>);

impl Stderr {
    pub(crate) fn new() -> Self {
        // SAFETY: no need to attach on unix
        Self(unsafe { AsyncFd::new_unchecked(StaticFd(libc::STDERR_FILENO)) })
    }
}

impl AsyncWrite for Stderr {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

impl AsRawFd for Stderr {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}
