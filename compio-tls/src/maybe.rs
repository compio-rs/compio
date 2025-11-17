#[cfg(any(feature = "native-tls", feature = "rustls"))]
use {
    crate::TlsStream,
    compio_buf::{BufResult, IoBuf, IoBufMut},
    compio_io::{AsyncRead, AsyncWrite},
    std::io,
};

/// Stream that can be either plain TCP or TLS-encrypted
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum MaybeTlsStream<S> {
    /// Plain, unencrypted stream
    Plain(S),
    /// TLS-encrypted stream
    #[cfg(any(feature = "native-tls", feature = "rustls"))]
    Tls(TlsStream<S>),
}

impl<S> MaybeTlsStream<S> {
    /// Create an unencrypted stream.
    pub fn plain(stream: S) -> Self {
        MaybeTlsStream::Plain(stream)
    }

    /// Create a TLS-encrypted stream.
    #[cfg(any(feature = "native-tls", feature = "rustls"))]
    pub fn tls(stream: TlsStream<S>) -> Self {
        MaybeTlsStream::Tls(stream)
    }

    /// Whether the stream is TLS-encrypted.
    pub fn is_tls(&self) -> bool {
        #[cfg(any(feature = "native-tls", feature = "rustls"))]
        {
            matches!(self, MaybeTlsStream::Tls(_))
        }
        #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
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
            #[cfg(any(feature = "native-tls", feature = "rustls"))]
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
            #[cfg(any(feature = "native-tls", feature = "rustls"))]
            MaybeTlsStream::Tls(stream) => stream.write(buf).await,
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.flush().await,
            #[cfg(any(feature = "native-tls", feature = "rustls"))]
            MaybeTlsStream::Tls(stream) => stream.flush().await,
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            MaybeTlsStream::Plain(stream) => stream.shutdown().await,
            #[cfg(any(feature = "native-tls", feature = "rustls"))]
            MaybeTlsStream::Tls(stream) => stream.shutdown().await,
        }
    }
}
