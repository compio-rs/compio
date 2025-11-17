use std::io;

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{AsyncRead, AsyncWrite};

use crate::TlsStream;

/// Stream that can be either plain TCP or TLS-encrypted
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum MaybeTlsStream<S> {
    /// Plain, unencrypted stream
    Plain(S),
    /// TLS-encrypted stream
    Tls(TlsStream<S>),
}

impl<S> MaybeTlsStream<S> {
    /// Create an unencrypted stream.
    pub fn plain(stream: S) -> Self {
        MaybeTlsStream::Plain(stream)
    }

    /// Create a TLS-encrypted stream.
    pub fn tls(stream: TlsStream<S>) -> Self {
        MaybeTlsStream::Tls(stream)
    }

    /// Whether the stream is TLS-encrypted.
    pub fn is_tls(&self) -> bool {
        matches!(self, MaybeTlsStream::Tls(_))
    }
}

impl<S> AsyncRead for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.read(buf).await,
            MaybeTlsStream::Tls(stream) => stream.read(buf).await,
        }
    }
}

impl<S> AsyncWrite for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.write(buf).await,
            MaybeTlsStream::Tls(stream) => stream.write(buf).await,
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.flush().await,
            MaybeTlsStream::Tls(stream) => stream.flush().await,
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.shutdown().await,
            MaybeTlsStream::Tls(stream) => stream.shutdown().await,
        }
    }
}
