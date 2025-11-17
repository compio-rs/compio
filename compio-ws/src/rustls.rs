//! Rustls support.

#[cfg(any(feature = "rustls-platform-verifier", feature = "webpki-roots"))]
use std::sync::Arc;

use compio_io::{AsyncRead, AsyncWrite};
use compio_net::TcpStream;
use compio_tls::TlsConnector;
#[cfg(any(feature = "rustls-platform-verifier", feature = "webpki-roots"))]
use rustls::{ClientConfig, RootCertStore};
use tungstenite::{
    Error,
    client::{IntoClientRequest, uri_mode},
    handshake::client::{Request, Response},
    stream::Mode,
};

use crate::{
    WebSocketConfig, WebSocketStream, client_async_with_config, domain, stream::MaybeTlsStream,
};

/// Type alias for a stream that can be either plain TCP or TLS-encrypted.
pub type AutoStream<S> = MaybeTlsStream<S>;

/// Type alias for a TLS connector.
pub type Connector = TlsConnector;

async fn wrap_stream<S>(
    socket: S,
    domain: String,
    connector: Option<Connector>,
    mode: Mode,
) -> Result<AutoStream<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    match mode {
        Mode::Plain => Ok(MaybeTlsStream::Plain(socket)),
        Mode::Tls => {
            let stream = {
                let connector = if let Some(connector) = connector {
                    connector
                } else {
                    // Create TLS connector with platform verifier when feature is enabled
                    #[cfg(feature = "rustls-platform-verifier")]
                    {
                        use rustls_platform_verifier::BuilderVerifierExt;

                        // Use platform's native certificate verification
                        // This provides better security and enterprise integration
                        let config_result = ClientConfig::builder().with_platform_verifier();

                        match config_result {
                            Ok(config_builder) => {
                                log::debug!(
                                    "Using rustls-platform-verifier for certificate validation"
                                );
                                TlsConnector::from(Arc::new(config_builder.with_no_client_auth()))
                            }
                            Err(e) => {
                                log::warn!("Error creating platform verifier: {e}");

                                // Only fail if webpki-roots is NOT enabled as fallback
                                #[cfg(not(feature = "webpki-roots"))]
                                {
                                    return Err(Error::Io(std::io::Error::new(
                                        std::io::ErrorKind::Other,
                                        format!("Failed to create platform verifier: {}", e),
                                    )));
                                }

                                // Fall through to webpki-roots if available
                                #[cfg(feature = "webpki-roots")]
                                {
                                    use log::debug;

                                    let mut root_store = RootCertStore::empty();
                                    let webpki_certs = webpki_roots::TLS_SERVER_ROOTS.to_vec();
                                    root_store.extend(webpki_certs);
                                    debug!(
                                        "Falling back to {} webpki root certificates",
                                        webpki_roots::TLS_SERVER_ROOTS.len()
                                    );

                                    TlsConnector::from(Arc::new(
                                        ClientConfig::builder()
                                            .with_root_certificates(root_store)
                                            .with_no_client_auth(),
                                    ))
                                }
                            }
                        }
                    }

                    // Use webpki-roots when platform-verifier is not available
                    // This serves as a fallback or standalone certificate source
                    #[cfg(all(
                        feature = "webpki-roots",
                        not(feature = "rustls-platform-verifier")
                    ))]
                    {
                        use log::debug;

                        let mut root_store = RootCertStore::empty();
                        let webpki_certs = webpki_roots::TLS_SERVER_ROOTS.to_vec();
                        root_store.extend(webpki_certs);
                        debug!(
                            "Using {} webpki root certificates",
                            webpki_roots::TLS_SERVER_ROOTS.len()
                        );

                        TlsConnector::from(Arc::new(
                            ClientConfig::builder()
                                .with_root_certificates(root_store)
                                .with_no_client_auth(),
                        ))
                    }

                    // Check if we have neither feature enabled
                    #[cfg(not(any(
                        feature = "rustls-platform-verifier",
                        feature = "webpki-roots"
                    )))]
                    {
                        return Err(Error::Io(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "No root certificate features enabled. Enable either \
                             'rustls-platform-verifier' or 'webpki-roots'",
                        )));
                    }
                };

                connector
                    .connect(&domain, socket)
                    .await
                    .map_err(Error::Io)?
            };
            Ok(MaybeTlsStream::Tls(stream))
        }
    }
}

/// Creates a WebSocket handshake from a request and a stream,
/// upgrading the stream to TLS if required.
pub async fn client_async_tls<R, S>(
    request: R,
    stream: S,
) -> Result<(WebSocketStream<AutoStream<S>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    AutoStream<S>: Unpin,
{
    client_async_tls_with_connector_and_config(request, stream, None, None).await
}

/// The same as `client_async_tls()` but the one can specify a websocket
/// configuration.
pub async fn client_async_tls_with_config<R, S>(
    request: R,
    stream: S,
    config: Option<WebSocketConfig>,
) -> Result<(WebSocketStream<AutoStream<S>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    AutoStream<S>: Unpin,
{
    client_async_tls_with_connector_and_config(request, stream, None, config).await
}

/// The same as `client_async_tls()` but the one can specify a connector.
pub async fn client_async_tls_with_connector<R, S>(
    request: R,
    stream: S,
    connector: Option<Connector>,
) -> Result<(WebSocketStream<AutoStream<S>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    AutoStream<S>: Unpin,
{
    client_async_tls_with_connector_and_config(request, stream, connector, None).await
}

/// The same as `client_async_tls()` but the one can specify a websocket
/// configuration, and an optional connector.
pub async fn client_async_tls_with_connector_and_config<R, S>(
    request: R,
    stream: S,
    connector: Option<Connector>,
    config: Option<WebSocketConfig>,
) -> Result<(WebSocketStream<AutoStream<S>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    AutoStream<S>: Unpin,
{
    let request: Request = request.into_client_request()?;

    let domain = domain(&request)?;

    let mode = uri_mode(request.uri())?;

    let stream = wrap_stream(stream, domain, connector, mode).await?;
    client_async_with_config(request, stream, config).await
}

/// Type alias for a connect stream.
pub type ConnectStream = AutoStream<TcpStream>;

/// Connect to a given URL.
pub async fn connect_async<R>(
    request: R,
) -> Result<(WebSocketStream<ConnectStream>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    connect_async_with_config(request, None, false).await
}

/// The same as `connect_async()` but the one can specify a websocket
/// configuration. `disable_nagle` specifies if the Nagle's algorithm must be
/// disabled, i.e. `set_nodelay(true)`. If you don't know what the Nagle's
/// algorithm is, better leave it to `false`.
pub async fn connect_async_with_config<R>(
    request: R,
    config: Option<WebSocketConfig>,
    disable_nagle: bool,
) -> Result<(WebSocketStream<ConnectStream>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    let request: Request = request.into_client_request()?;

    let domain = domain(&request)?;
    let port = port(&request)?;

    let socket = TcpStream::connect((domain.as_str(), port))
        .await
        .map_err(Error::Io)?;

    if disable_nagle {
        socket.set_nodelay(true).map_err(Error::Io)?;
    }

    client_async_tls_with_connector_and_config(request, socket, None, config).await
}

/// The same as `connect_async()` but the one can specify a TLS connector.
pub async fn connect_async_with_tls_connector<R>(
    request: R,
    connector: Option<Connector>,
) -> Result<(WebSocketStream<ConnectStream>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    connect_async_with_tls_connector_and_config(request, connector, None).await
}

/// The same as `connect_async()` but the one can specify a websocket
/// configuration, a TLS connector, and whether to disable Nagle's algorithm.
pub async fn connect_async_with_tls_connector_and_config<R>(
    request: R,
    connector: Option<Connector>,
    config: Option<WebSocketConfig>,
) -> Result<(WebSocketStream<ConnectStream>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    let request: Request = request.into_client_request()?;

    let domain = domain(&request)?;
    let port = port(&request)?;

    let socket = TcpStream::connect((domain.as_str(), port))
        .await
        .map_err(Error::Io)?;
    client_async_tls_with_connector_and_config(request, socket, connector, config).await
}

#[inline]
#[allow(clippy::result_large_err)]
fn port(request: &Request) -> Result<u16, Error> {
    request
        .uri()
        .port_u16()
        .or_else(|| match uri_mode(request.uri()).ok()? {
            Mode::Plain => Some(80),
            Mode::Tls => Some(443),
        })
        .ok_or(Error::Url(
            tungstenite::error::UrlError::UnsupportedUrlScheme,
        ))
}
