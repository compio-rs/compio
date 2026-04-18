use std::{
    borrow::Cow,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf};
use compio_io::{AsyncRead, AsyncWrite, compat::AsyncStream, util::Splittable};

use crate::{TlsStream, read_futures};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum MaybeTlsStreamInner<S: Splittable> {
    Plain(Pin<Box<AsyncStream<S>>>),
    Tls(TlsStream<S>),
}

/// Stream that can be either plain TCP or TLS-encrypted, with compatibility for
/// [`futures_util`].
#[derive(Debug)]
pub struct MaybeTlsStream<S: Splittable>(MaybeTlsStreamInner<S>);

impl<S: Splittable> MaybeTlsStream<S> {
    /// Create an unencrypted stream.
    pub fn new_plain(stream: S) -> Self {
        Self(MaybeTlsStreamInner::Plain(Box::pin(AsyncStream::new(
            stream,
        ))))
    }

    /// Create a TLS-encrypted stream.
    pub fn new_tls(stream: TlsStream<S>) -> Self {
        Self(MaybeTlsStreamInner::Tls(stream))
    }

    /// Whether the stream is TLS-encrypted.
    pub fn is_tls(&self) -> bool {
        matches!(self.0, MaybeTlsStreamInner::Tls(_))
    }
}

impl<S: Splittable + 'static> MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    /// Returns the negotiated ALPN protocol.
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match &self.0 {
            MaybeTlsStreamInner::Plain(_) => None,
            MaybeTlsStreamInner::Tls(s) => s.negotiated_alpn(),
        }
    }
}

impl<S: Splittable + 'static> futures_util::AsyncRead for MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            MaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            MaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl<S: Splittable + 'static> AsyncRead for MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        read_futures(self, buf).await
    }
}

impl<S: Splittable + 'static> futures_util::AsyncWrite for MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            MaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            MaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            MaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            MaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            MaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_flush(cx),
            MaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            MaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_close(cx),
            MaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_close(cx),
        }
    }
}

impl<S: Splittable + 'static> AsyncWrite for MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slice = buf.as_init();
        let res = futures_util::AsyncWriteExt::write(self, slice).await;
        BufResult(res, buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slices = buf.iter_slice().map(io::IoSlice::new).collect::<Vec<_>>();
        let res = futures_util::AsyncWriteExt::write_vectored(self, &slices).await;
        BufResult(res, buf)
    }

    async fn flush(&mut self) -> io::Result<()> {
        futures_util::AsyncWriteExt::flush(self).await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        futures_util::AsyncWriteExt::close(self).await
    }
}
