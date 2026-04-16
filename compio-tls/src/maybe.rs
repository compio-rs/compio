use std::{
    borrow::Cow,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{AsyncRead, AsyncWrite, compat::AsyncStream, util::Splittable};

use crate::TlsStream;

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum MaybeTlsStreamInner<S: Splittable> {
    /// Plain, unencrypted stream
    Plain(S),
    /// TLS-encrypted stream
    Tls(TlsStream<S>),
}

/// Stream that can be either plain TCP or TLS-encrypted
#[derive(Debug)]
pub struct MaybeTlsStream<S: Splittable>(MaybeTlsStreamInner<S>);

impl<S: Splittable> MaybeTlsStream<S> {
    /// Create an unencrypted stream.
    pub fn new_plain(stream: S) -> Self {
        Self(MaybeTlsStreamInner::Plain(stream))
    }

    /// Create a TLS-encrypted stream.
    pub fn new_tls(stream: TlsStream<S>) -> Self {
        Self(MaybeTlsStreamInner::Tls(stream))
    }

    /// Whether the stream is TLS-encrypted.
    pub fn is_tls(&self) -> bool {
        matches!(self.0, MaybeTlsStreamInner::Tls(_))
    }

    /// Convert this stream into a compatibility wrapper for [`futures_util`].
    pub fn into_compat(self) -> CompatMaybeTlsStream<S> {
        match self.0 {
            MaybeTlsStreamInner::Plain(stream) => CompatMaybeTlsStream::new_plain(stream),
            MaybeTlsStreamInner::Tls(stream) => CompatMaybeTlsStream::new_tls(stream),
        }
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

impl<S: Splittable + AsyncRead + AsyncWrite + 'static> AsyncRead for MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        match &mut self.0 {
            MaybeTlsStreamInner::Plain(stream) => stream.read(buf).await,
            MaybeTlsStreamInner::Tls(stream) => stream.read(buf).await,
        }
    }
}

impl<S: Splittable + AsyncRead + AsyncWrite + 'static> AsyncWrite for MaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        match &mut self.0 {
            MaybeTlsStreamInner::Plain(stream) => stream.write(buf).await,
            MaybeTlsStreamInner::Tls(stream) => stream.write(buf).await,
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            MaybeTlsStreamInner::Plain(stream) => stream.flush().await,
            MaybeTlsStreamInner::Tls(stream) => stream.flush().await,
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match &mut self.0 {
            MaybeTlsStreamInner::Plain(stream) => stream.shutdown().await,
            MaybeTlsStreamInner::Tls(stream) => stream.shutdown().await,
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum CompatMaybeTlsStreamInner<S: Splittable> {
    Plain(Pin<Box<AsyncStream<S>>>),
    Tls(TlsStream<S>),
}

/// Stream that can be either plain TCP or TLS-encrypted, with compatibility for
/// [`futures_util`].
#[derive(Debug)]
pub struct CompatMaybeTlsStream<S: Splittable>(CompatMaybeTlsStreamInner<S>);

impl<S: Splittable> CompatMaybeTlsStream<S> {
    /// Create an unencrypted stream.
    pub fn new_plain(stream: S) -> Self {
        Self(CompatMaybeTlsStreamInner::Plain(Box::pin(
            AsyncStream::new(stream),
        )))
    }

    /// Create a TLS-encrypted stream.
    pub fn new_tls(stream: TlsStream<S>) -> Self {
        Self(CompatMaybeTlsStreamInner::Tls(stream))
    }

    /// Whether the stream is TLS-encrypted.
    pub fn is_tls(&self) -> bool {
        matches!(self.0, CompatMaybeTlsStreamInner::Tls(_))
    }
}

impl<S: Splittable + 'static> CompatMaybeTlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    /// Returns the negotiated ALPN protocol.
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match &self.0 {
            CompatMaybeTlsStreamInner::Plain(_) => None,
            CompatMaybeTlsStreamInner::Tls(s) => s.negotiated_alpn(),
        }
    }
}

impl<S: Splittable + 'static> futures_util::AsyncRead for CompatMaybeTlsStream<S>
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
            CompatMaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            CompatMaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl<S: Splittable + 'static> futures_util::AsyncWrite for CompatMaybeTlsStream<S>
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
            CompatMaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            CompatMaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            CompatMaybeTlsStreamInner::Plain(stream) => {
                Pin::new(stream).poll_write_vectored(cx, bufs)
            }
            CompatMaybeTlsStreamInner::Tls(stream) => {
                Pin::new(stream).poll_write_vectored(cx, bufs)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            CompatMaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_flush(cx),
            CompatMaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            CompatMaybeTlsStreamInner::Plain(stream) => Pin::new(stream).poll_close(cx),
            CompatMaybeTlsStreamInner::Tls(stream) => Pin::new(stream).poll_close(cx),
        }
    }
}
