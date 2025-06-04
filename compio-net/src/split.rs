use std::{error::Error, fmt, io};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::AsRawFd;
use compio_io::{AsyncRead, AsyncWrite, AsyncWriteAt};

pub(crate) fn split<T>(stream: &T) -> (ReadHalf<'_, T>, WriteHalf<'_, T>)
where
    for<'a> &'a T: AsyncRead + AsyncWrite,
{
    (ReadHalf(stream), WriteHalf(stream))
}

/// Borrowed read half.
#[derive(Debug)]
pub struct ReadHalf<'a, T>(&'a T);

impl<T> AsyncRead for ReadHalf<'_, T>
where
    for<'a> &'a T: AsyncRead,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.0.read_vectored(buf).await
    }
}

/// Borrowed write half.
#[derive(Debug)]
pub struct WriteHalf<'a, T>(&'a T);

impl<T> AsyncWrite for WriteHalf<'_, T>
where
    for<'a> &'a T: AsyncWrite,
{
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.write(buf).await
    }

    async fn write_vectored<B: IoVectoredBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

pub(crate) fn into_split<T>(stream: T) -> (OwnedReadHalf<T>, OwnedWriteHalf<T>)
where
    for<'a> &'a T: AsyncRead + AsyncWrite,
    T: Clone,
{
    (OwnedReadHalf(stream.clone()), OwnedWriteHalf(stream))
}

/// Owned read half.
#[derive(Debug)]
pub struct OwnedReadHalf<T>(T);

impl<T: AsRawFd> OwnedReadHalf<T> {
    /// Attempts to put the two halves of a `TcpStream` back together and
    /// recover the original socket. Succeeds only if the two halves
    /// originated from the same call to `into_split`.
    pub fn reunite(self, w: OwnedWriteHalf<T>) -> Result<T, ReuniteError<T>> {
        if self.0.as_raw_fd() == w.0.as_raw_fd() {
            drop(w);
            Ok(self.0)
        } else {
            Err(ReuniteError(self, w))
        }
    }
}

impl<T> AsyncRead for OwnedReadHalf<T>
where
    for<'a> &'a T: AsyncRead,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&self.0).read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        (&self.0).read_vectored(buf).await
    }
}

/// Owned write half.
#[derive(Debug)]
pub struct OwnedWriteHalf<T>(T);

impl<T> AsyncWrite for OwnedWriteHalf<T>
where
    for<'a> &'a T: AsyncWrite,
{
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        (&self.0).write(buf).await
    }

    async fn write_vectored<B: IoVectoredBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        (&self.0).write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        (&self.0).flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        (&self.0).shutdown().await
    }
}

/// Error indicating that two halves were not from the same socket, and thus
/// could not be reunited.
#[derive(Debug)]
pub struct ReuniteError<T>(pub OwnedReadHalf<T>, pub OwnedWriteHalf<T>);

impl<T> fmt::Display for ReuniteError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tried to reunite halves that are not from the same socket"
        )
    }
}

impl<T: fmt::Debug> Error for ReuniteError<T> {}
