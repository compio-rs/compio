use std::{io, ops::Deref, sync::Arc};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_io::{AsyncRead, AsyncWrite};

pub(crate) fn split<T>(stream: &T) -> (ReadHalf<T>, WriteHalf<T>)
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
{
    let stream = Arc::new(stream);
    (OwnedReadHalf(stream.clone()), OwnedWriteHalf(stream))
}

/// Owned read half.
#[derive(Debug)]
pub struct OwnedReadHalf<T>(Arc<T>);

impl<T> AsyncRead for OwnedReadHalf<T>
where
    for<'a> &'a T: AsyncRead,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.deref().read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.0.deref().read_vectored(buf).await
    }
}

/// Owned write half.
#[derive(Debug)]
pub struct OwnedWriteHalf<T>(Arc<T>);

impl<T> AsyncWrite for OwnedWriteHalf<T>
where
    for<'a> &'a T: AsyncWrite,
{
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.deref().write(buf).await
    }

    async fn write_vectored<B: IoVectoredBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.deref().write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.deref().flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.deref().shutdown().await
    }
}
