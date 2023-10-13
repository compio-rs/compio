use std::io::Result as IoResult;

use compio_buf::{BufResult, IoBuf, IoVectoredBuf};

/// # AsyncWrite
///
/// Async write with a ownership of a buffer
pub trait AsyncWrite {
    /// Write some bytes from the buffer into this source and return a
    /// [`BufResult`], consisting of the buffer and a [`usize`] indicating how
    /// many bytes were written.
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T>;

    /// Like `write`, except that it write bytes from a buffer implements
    /// [`IoVectoredBuf`] into the source.
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T>;
}

impl<A: AsyncWrite + ?Sized> AsyncWrite for &mut A {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).write_vectored(buf).await
    }
}

/// # AsyncWriteAt
///
/// Async write with a ownership of a buffer and a position
pub trait AsyncWriteAt {
    /// Like `write`, except that it writes at a specified position.
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T>;
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAt for &mut A {
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T> {
        (**self).write_at(buf, pos).await
    }
}

/// # AsyncBufWrite
///
/// Async write with buffered content
pub trait AsyncBufWrite: AsyncWrite {
    /// Try write data and get a reference to the internal buffer
    async fn flush_buf(&mut self) -> IoResult<()>;
}

impl<A: AsyncBufWrite + ?Sized> AsyncBufWrite for &mut A {
    async fn flush_buf(&mut self) -> IoResult<()> {
        (**self).flush_buf().await
    }
}
