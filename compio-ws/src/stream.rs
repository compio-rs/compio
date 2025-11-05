use std::io::Result as IoResult;

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{AsyncRead, AsyncWrite};
#[cfg(feature = "rustls")]
use compio_tls::TlsStream;

/// Stream that can be either plain TCP or TLS-encrypted
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum MaybeTlsStream<S> {
    /// Plain, unencrypted stream
    Plain(S),
    /// TLS-encrypted stream
    #[cfg(feature = "rustls")]
    Tls(TlsStream<S>),
}

impl<S> MaybeTlsStream<S> {
    pub fn plain(stream: S) -> Self {
        MaybeTlsStream::Plain(stream)
    }

    #[cfg(feature = "rustls")]
    pub fn tls(stream: TlsStream<S>) -> Self {
        MaybeTlsStream::Tls(stream)
    }

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

impl<S> Unpin for MaybeTlsStream<S> where S: Unpin {}
