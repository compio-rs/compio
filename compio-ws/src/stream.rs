//! Provides [`MaybeTlsStream`].

#[cfg(feature = "rustls")]
use std::io::Result as IoResult;

#[cfg(feature = "rustls")]
use compio_buf::{BufResult, IoBuf, IoBufMut};
#[cfg(feature = "rustls")]
use compio_io::{AsyncRead, AsyncWrite};
#[cfg(feature = "rustls")]
use compio_tls::TlsStream;

/// Stream that can be either plain TCP or TLS-encrypted
#[cfg(feature = "rustls")]
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum MaybeTlsStream<S> {
    /// Plain, unencrypted stream
    Plain(S),
    /// TLS-encrypted stream
    #[cfg(feature = "rustls")]
    Tls(TlsStream<S>),
}

#[cfg(feature = "rustls")]
impl<S> MaybeTlsStream<S> {
    /// Create an unencrypted stream.
    pub fn plain(stream: S) -> Self {
        MaybeTlsStream::Plain(stream)
    }

    /// Create a TLS-encrypted stream.
    #[cfg(feature = "rustls")]
    pub fn tls(stream: TlsStream<S>) -> Self {
        MaybeTlsStream::Tls(stream)
    }

    /// Whether the stream is TLS-encrypted.
    pub fn is_tls(&self) -> bool {
        #[cfg(feature = "rustls")]
        {
            matches!(self, MaybeTlsStream::Tls(_))
        }
        #[cfg(not(feature = "rustls"))]
        {
            false
        }
    }
}

#[cfg(feature = "rustls")]
impl<S> AsyncRead for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.read(buf).await,
            #[cfg(feature = "rustls")]
            MaybeTlsStream::Tls(stream) => stream.read(buf).await,
        }
    }
}

#[cfg(feature = "rustls")]
impl<S> AsyncWrite for MaybeTlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.write(buf).await,
            #[cfg(feature = "rustls")]
            MaybeTlsStream::Tls(stream) => stream.write(buf).await,
        }
    }

    async fn flush(&mut self) -> IoResult<()> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.flush().await,
            #[cfg(feature = "rustls")]
            MaybeTlsStream::Tls(stream) => stream.flush().await,
        }
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.shutdown().await,
            #[cfg(feature = "rustls")]
            MaybeTlsStream::Tls(stream) => stream.shutdown().await,
        }
    }
}
