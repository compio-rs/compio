use std::io;

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{AsyncRead, AsyncWrite};

use crate::TlsStream;

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum MaybeTlsStreamInner<S> {
    /// Plain, unencrypted stream
    Plain(S),
    /// TLS-encrypted stream
    Tls(TlsStream<S>),
}

/// Stream that can be either plain TCP or TLS-encrypted
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

impl<S> AsyncRead for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        match &mut self.0 {
            MaybeTlsStreamInner::Plain(stream) => stream.read(buf).await,
            MaybeTlsStreamInner::Tls(stream) => stream.read(buf).await,
        }
    }
}

impl<S> AsyncWrite for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
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
