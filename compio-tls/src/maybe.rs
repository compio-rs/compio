use std::{
    borrow::Cow,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{AsyncRead, AsyncWrite};

use crate::TlsStream;

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum MaybeTlsStreamInner<S> {
    Plain(S),
    Tls(TlsStream<S>),
}

/// A futures-based stream that can be either plain TCP or TLS-encrypted.
#[derive(Debug)]
pub struct MaybeTlsStream<S>(MaybeTlsStreamInner<S>);

impl<S> MaybeTlsStream<S> {
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
}

impl<S> MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Returns the negotiated ALPN protocol.
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match &self.0 {
            MaybeTlsStreamInner::Plain(_) => None,
            MaybeTlsStreamInner::Tls(s) => s.negotiated_alpn(),
        }
    }
}

impl<S> AsyncRead for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
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

impl<S> AsyncWrite for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
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
