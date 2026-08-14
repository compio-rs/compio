use std::{
    borrow::Cow,
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

#[cfg(feature = "py-dynamic-openssl")]
#[doc(hidden)]
impl<S> From<crate::py_ossl::TlsStream<S>> for TlsStream<S> {
    fn from(value: crate::py_ossl::TlsStream<S>) -> Self {
        Self(TlsStreamInner::PyDynamicOpenSsl(value))
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
