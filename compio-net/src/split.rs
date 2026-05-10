use std::{io, ops::Deref};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_io::{AsyncRead, AsyncWrite};

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

impl<T> Deref for ReadHalf<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
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
        (self.0).write(buf).await
    }

    async fn write_vectored<B: IoVectoredBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        (self.0).write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        (self.0).flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        (self.0).shutdown().await
    }
}

impl<T> Deref for WriteHalf<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}
