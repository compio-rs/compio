use std::{
    borrow::Cow,
    error::Error,
    fmt::Display,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{AsyncRead, AsyncWrite};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum TlsStreamInner<S> {
    #[cfg(feature = "native-tls")]
    NativeTls(crate::native::TlsStream<S>),
    #[cfg(feature = "rustls")]
    Rustls(futures_rustls::TlsStream<S>),
    #[cfg(feature = "py-dynamic-openssl")]
    PyDynamicOpenSsl(crate::py_ossl::TlsStream<S>),
    #[cfg(not(any(
        feature = "native-tls",
        feature = "rustls",
        feature = "py-dynamic-openssl",
    )))]
    None(std::convert::Infallible, std::marker::PhantomData<S>),
}

impl<S> TlsStreamInner<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => s.negotiated_alpn().ok().flatten().map(Cow::from),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => s.get_ref().1.alpn_protocol().map(Cow::from),
            #[cfg(feature = "py-dynamic-openssl")]
            Self::PyDynamicOpenSsl(s) => s.negotiated_alpn().map(Cow::from),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            Self::None(f, ..) => match *f {},
        }
    }
}

#[cfg(feature = "tokio")]
impl<S> TlsStream<S> {
    /// Whether `poll_write_vectored` takes all the buffers at once, not only
    /// the first: rustls copies them all into its own buffer.
    pub(crate) fn writes_vectored(&self) -> bool {
        cfg_select! {
            feature = "rustls" => matches!(self.0, TlsStreamInner::Rustls(_)),
            _ => false,
        }
    }
}

/// A wrapper around an underlying raw stream which implements the TLS or SSL
/// protocol.
///
/// A `TlsStream<S>` represents a handshake that has been completed successfully
/// and both the server and the client are ready for receiving and sending
/// data. Bytes read from a `TlsStream` are decrypted from `S` and bytes written
/// to a `TlsStream` are encrypted when passing through to `S`.
#[derive(Debug)]
pub struct TlsStream<S>(TlsStreamInner<S>);

impl<S> TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Returns the negotiated ALPN protocol.
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        self.0.negotiated_alpn()
    }
}

/// Error returned when converting a [`TlsStream`] reference into a
/// backend-specific connection fails.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryFromTlsStreamError {
    /// The stream is backed by a different TLS implementation.
    BackendMismatch,
    /// Expected a server connection, but found a client connection.
    #[cfg(feature = "rustls")]
    UnexpectedClient,
    /// Expected a client connection, but found a server connection.
    #[cfg(feature = "rustls")]
    UnexpectedServer,
}

impl Display for TryFromTlsStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TryFromTlsStreamError::BackendMismatch => {
                f.write_str("stream is backed by a different TLS implementation")
            }
            #[cfg(feature = "rustls")]
            TryFromTlsStreamError::UnexpectedClient => {
                f.write_str("expected a server connection, but found a client connection")
            }
            #[cfg(feature = "rustls")]
            TryFromTlsStreamError::UnexpectedServer => {
                f.write_str("expected a client connection, but found a server connection")
            }
        }
    }
}

impl Error for TryFromTlsStreamError {}

impl From<TryFromTlsStreamError> for std::io::Error {
    fn from(value: TryFromTlsStreamError) -> Self {
        io::Error::new(io::ErrorKind::InvalidInput, value)
    }
}

#[cfg(feature = "native-tls")]
#[doc(hidden)]
impl<S> From<crate::native::TlsStream<S>> for TlsStream<S> {
    fn from(value: crate::native::TlsStream<S>) -> Self {
        Self(TlsStreamInner::NativeTls(value))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S> From<futures_rustls::client::TlsStream<S>> for TlsStream<S> {
    fn from(value: futures_rustls::client::TlsStream<S>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Client(
            value,
        )))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S> From<futures_rustls::server::TlsStream<S>> for TlsStream<S> {
    fn from(value: futures_rustls::server::TlsStream<S>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Server(
            value,
        )))
    }
}

#[cfg(feature = "rustls")]
impl<'a, S> TryFrom<&'a TlsStream<S>> for &'a rustls::ServerConnection {
    type Error = TryFromTlsStreamError;

    fn try_from(value: &'a TlsStream<S>) -> Result<Self, Self::Error> {
        match &value.0 {
            TlsStreamInner::Rustls(tls_stream) => match tls_stream {
                futures_rustls::TlsStream::Server(tls_stream) => Ok(tls_stream.get_ref().1),
                futures_rustls::TlsStream::Client(_) => {
                    Err(TryFromTlsStreamError::UnexpectedClient)
                }
            },
            #[allow(unreachable_patterns)]
            _ => Err(TryFromTlsStreamError::BackendMismatch),
        }
    }
}

#[cfg(feature = "rustls")]
impl<'a, S> TryFrom<&'a TlsStream<S>> for &'a rustls::ClientConnection {
    type Error = TryFromTlsStreamError;

    fn try_from(value: &'a TlsStream<S>) -> Result<Self, Self::Error> {
        match &value.0 {
            TlsStreamInner::Rustls(tls_stream) => match tls_stream {
                futures_rustls::TlsStream::Client(tls_stream) => Ok(tls_stream.get_ref().1),
                futures_rustls::TlsStream::Server(_) => {
                    Err(TryFromTlsStreamError::UnexpectedServer)
                }
            },
            #[allow(unreachable_patterns)]
            _ => Err(TryFromTlsStreamError::BackendMismatch),
        }
    }
}

#[cfg(feature = "rustls")]
impl<'a, S> TryFrom<&'a TlsStream<S>> for &'a rustls::CommonState {
    type Error = TryFromTlsStreamError;

    fn try_from(value: &'a TlsStream<S>) -> Result<Self, Self::Error> {
        match &value.0 {
            TlsStreamInner::Rustls(tls_stream) => Ok(tls_stream.get_ref().1),
            #[allow(unreachable_patterns)]
            _ => Err(TryFromTlsStreamError::BackendMismatch),
        }
    }
}

#[cfg(feature = "py-dynamic-openssl")]
#[doc(hidden)]
impl<S> From<crate::py_ossl::TlsStream<S>> for TlsStream<S> {
    fn from(value: crate::py_ossl::TlsStream<S>) -> Self {
        Self(TlsStreamInner::PyDynamicOpenSsl(value))
    }
}

#[cfg(feature = "py-dynamic-openssl")]
impl<'a, S> TryFrom<&'a TlsStream<S>> for &'a compio_py_dynamic_openssl::ssl::Ssl {
    type Error = TryFromTlsStreamError;

    fn try_from(value: &'a TlsStream<S>) -> Result<Self, Self::Error> {
        match &value.0 {
            TlsStreamInner::PyDynamicOpenSsl(tls_stream) => Ok(tls_stream.get_ref().ssl()),
            #[allow(unreachable_patterns)]
            _ => Err(TryFromTlsStreamError::BackendMismatch),
        }
    }
}

impl<S> AsyncRead for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}

impl<S> AsyncWrite for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_flush(cx),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_close(cx),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_close(cx),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_close(cx),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}
