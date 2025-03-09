use std::io;

use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};

use crate::TlsStream;

#[cfg(feature = "rustls")]
mod rtls;

#[derive(Debug, Clone)]
enum TlsConnectorInner {
    #[cfg(feature = "native-tls")]
    NativeTls(native_tls::TlsConnector),
    #[cfg(feature = "rustls")]
    Rustls(rtls::TlsConnector),
}

/// A wrapper around a [`native_tls::TlsConnector`] or [`rustls::ClientConfig`],
/// providing an async `connect` method.
#[derive(Debug, Clone)]
pub struct TlsConnector(TlsConnectorInner);

#[cfg(feature = "native-tls")]
impl From<native_tls::TlsConnector> for TlsConnector {
    fn from(value: native_tls::TlsConnector) -> Self {
        Self(TlsConnectorInner::NativeTls(value))
    }
}

#[cfg(feature = "rustls")]
impl From<std::sync::Arc<rustls::ClientConfig>> for TlsConnector {
    fn from(value: std::sync::Arc<rustls::ClientConfig>) -> Self {
        Self(TlsConnectorInner::Rustls(rtls::TlsConnector(value)))
    }
}

impl TlsConnector {
    /// Connects the provided stream with this connector, assuming the provided
    /// domain.
    ///
    /// This function will internally call `TlsConnector::connect` to connect
    /// the stream and returns a future representing the resolution of the
    /// connection operation. The returned future will resolve to either
    /// `TlsStream<S>` or `Error` depending if it's successful or not.
    ///
    /// This is typically used for clients who have already established, for
    /// example, a TCP connection to a remote server. That stream is then
    /// provided here to perform the client half of a connection to a
    /// TLS-powered server.
    pub async fn connect<S: AsyncRead + AsyncWrite>(
        &self,
        domain: &str,
        stream: S,
    ) -> io::Result<TlsStream<S>> {
        match &self.0 {
            #[cfg(feature = "native-tls")]
            TlsConnectorInner::NativeTls(c) => {
                handshake_native_tls(c.connect(domain, SyncStream::new(stream))).await
            }
            #[cfg(feature = "rustls")]
            TlsConnectorInner::Rustls(c) => handshake_rustls(c.connect(domain, stream)).await,
        }
    }
}

#[derive(Clone)]
enum TlsAcceptorInner {
    #[cfg(feature = "native-tls")]
    NativeTls(native_tls::TlsAcceptor),
    #[cfg(feature = "rustls")]
    Rustls(rtls::TlsAcceptor),
}

/// A wrapper around a [`native_tls::TlsAcceptor`] or [`rustls::ServerConfig`],
/// providing an async `accept` method.
#[derive(Clone)]
pub struct TlsAcceptor(TlsAcceptorInner);

#[cfg(feature = "native-tls")]
impl From<native_tls::TlsAcceptor> for TlsAcceptor {
    fn from(value: native_tls::TlsAcceptor) -> Self {
        Self(TlsAcceptorInner::NativeTls(value))
    }
}

#[cfg(feature = "rustls")]
impl From<std::sync::Arc<rustls::ServerConfig>> for TlsAcceptor {
    fn from(value: std::sync::Arc<rustls::ServerConfig>) -> Self {
        Self(TlsAcceptorInner::Rustls(rtls::TlsAcceptor(value)))
    }
}

impl TlsAcceptor {
    /// Accepts a new client connection with the provided stream.
    ///
    /// This function will internally call `TlsAcceptor::accept` to connect
    /// the stream and returns a future representing the resolution of the
    /// connection operation. The returned future will resolve to either
    /// `TlsStream<S>` or `Error` depending if it's successful or not.
    ///
    /// This is typically used after a new socket has been accepted from a
    /// `TcpListener`. That socket is then passed to this function to perform
    /// the server half of accepting a client connection.
    pub async fn accept<S: AsyncRead + AsyncWrite>(&self, stream: S) -> io::Result<TlsStream<S>> {
        match &self.0 {
            #[cfg(feature = "native-tls")]
            TlsAcceptorInner::NativeTls(c) => {
                handshake_native_tls(c.accept(SyncStream::new(stream))).await
            }
            #[cfg(feature = "rustls")]
            TlsAcceptorInner::Rustls(c) => handshake_rustls(c.accept(stream)).await,
        }
    }
}

#[cfg(feature = "native-tls")]
async fn handshake_native_tls<S: AsyncRead + AsyncWrite>(
    mut res: Result<
        native_tls::TlsStream<SyncStream<S>>,
        native_tls::HandshakeError<SyncStream<S>>,
    >,
) -> io::Result<TlsStream<S>> {
    use native_tls::HandshakeError;

    loop {
        match res {
            Ok(mut s) => {
                s.get_mut().flush_write_buf().await?;
                return Ok(TlsStream::from(s));
            }
            Err(e) => match e {
                HandshakeError::Failure(e) => return Err(io::Error::other(e)),
                HandshakeError::WouldBlock(mut mid_stream) => {
                    if mid_stream.get_mut().flush_write_buf().await? == 0 {
                        mid_stream.get_mut().fill_read_buf().await?;
                    }
                    res = mid_stream.handshake();
                }
            },
        }
    }
}

#[cfg(feature = "rustls")]
async fn handshake_rustls<S: AsyncRead + AsyncWrite, C, D>(
    mut res: Result<TlsStream<S>, rtls::HandshakeError<S, C>>,
) -> io::Result<TlsStream<S>>
where
    C: std::ops::DerefMut<Target = rustls::ConnectionCommon<D>>,
{
    use rtls::HandshakeError;

    loop {
        match res {
            Ok(mut s) => {
                s.flush().await?;
                return Ok(s);
            }
            Err(e) => match e {
                HandshakeError::Rustls(e) => return Err(io::Error::other(e)),
                HandshakeError::System(e) => return Err(e),
                HandshakeError::WouldBlock(mut mid_stream) => {
                    if mid_stream.get_mut().flush_write_buf().await? == 0 {
                        mid_stream.get_mut().fill_read_buf().await?;
                    }
                    res = mid_stream.handshake::<D>();
                }
            },
        }
    }
}
