//! TLS support for WebSocket connections (native-tls and rustls).

use compio_io::{AsyncRead, AsyncWrite};
use compio_net::{TcpOpts, TcpStream};
use compio_tls::{MaybeTlsStream, TlsConnector};
use tungstenite::{
    Error,
    client::{IntoClientRequest, uri_mode},
    handshake::client::{Request, Response},
    stream::Mode,
};

use crate::{WebSocketConfig, WebSocketStream, client_async_with_config};

mod encryption {
    #[cfg(feature = "native-tls")]
    pub mod native_tls {
        use compio_tls::TlsConnector;
        use tungstenite::{Error, error::TlsError};

        pub fn new_connector() -> Result<TlsConnector, Error> {
            let native_connector = native_tls::TlsConnector::new().map_err(TlsError::from)?;
            Ok(TlsConnector::from(native_connector))
        }
    }

    #[cfg(feature = "rustls")]
    pub mod rustls {
        use std::sync::Arc;

        use compio_tls::TlsConnector;
        pub use rustls::ClientConfig;
        use rustls::RootCertStore;
        use tungstenite::Error;

        fn config_with_certs() -> Result<Arc<ClientConfig>, Error> {
            #[allow(unused_mut)]
            let mut root_store = RootCertStore::empty();
            #[cfg(feature = "rustls-native-certs")]
            {
                let rustls_native_certs::CertificateResult { certs, errors, .. } =
                    rustls_native_certs::load_native_certs();

                if !errors.is_empty() {
                    compio_log::warn!("native root CA certificate loading errors: {errors:?}");
                }

                // Not finding any native root CA certificates is not fatal
                // if the "webpki-roots" feature is enabled.
                #[cfg(not(feature = "webpki-roots"))]
                if certs.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("no native root CA certificates found (errors: {errors:?})"),
                    )
                    .into());
                }

                let total_number = certs.len();
                let (number_added, number_ignored) = root_store.add_parsable_certificates(certs);
                compio_log::debug!(
                    "Added {number_added}/{total_number} native root certificates (ignored \
                     {number_ignored})"
                );
            }
            #[cfg(feature = "webpki-roots")]
            {
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }

            Ok(Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth(),
            ))
        }

        #[cfg(feature = "rustls-platform-verifier")]
        fn config_with_platform_verifier() -> Result<Arc<ClientConfig>, Error> {
            use rustls_platform_verifier::BuilderVerifierExt;

            // Use platform's native certificate verification
            // This provides better security and enterprise integration
            let config_result = ClientConfig::builder()
                .with_platform_verifier()
                .map_err(tungstenite::error::TlsError::from)?;
            Ok(Arc::new(config_result.with_no_client_auth()))
        }

        pub fn new_connector() -> Result<TlsConnector, Error> {
            // Create TLS connector with platform verifier when feature is enabled
            #[cfg(feature = "rustls-platform-verifier")]
            {
                let config = match config_with_platform_verifier() {
                    Ok(config_builder) => config_builder,
                    Err(_e) => {
                        compio_log::warn!("Error creating platform verifier: {_e}");
                        config_with_certs()?
                    }
                };
                Ok(TlsConnector::from(config))
            }
            #[cfg(not(feature = "rustls-platform-verifier"))]
            {
                // Create TLS connector with certs from enabled features
                let config = config_with_certs()?;
                Ok(TlsConnector::from(config))
            }
        }
    }
}

async fn wrap_stream<S>(
    socket: S,
    domain: &str,
    connector: Option<TlsConnector>,
    mode: Mode,
) -> Result<MaybeTlsStream<S>, Error>
where
    S: AsyncRead + AsyncWrite + 'static,
{
    match mode {
        Mode::Plain => Ok(MaybeTlsStream::new_plain(socket)),
        Mode::Tls => {
            let stream = {
                let connector = if let Some(connector) = connector {
                    connector
                } else {
                    #[cfg(feature = "native-tls")]
                    {
                        match encryption::native_tls::new_connector() {
                            Ok(c) => c,
                            Err(_e) => {
                                compio_log::warn!(
                                    "Falling back to rustls TLS connector due to native-tls \
                                     error: {}",
                                    _e
                                );
                                #[cfg(feature = "rustls")]
                                {
                                    encryption::rustls::new_connector()?
                                }
                                #[cfg(not(feature = "rustls"))]
                                {
                                    return Err(_e);
                                }
                            }
                        }
                    }
                    #[cfg(all(feature = "rustls", not(feature = "native-tls")))]
                    {
                        encryption::rustls::new_connector()?
                    }
                    #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
                    {
                        return Err(Error::Url(
                            tungstenite::error::UrlError::TlsFeatureNotEnabled,
                        ));
                    }
                };

                connector.connect(domain, socket).await.map_err(Error::Io)?
            };
            Ok(MaybeTlsStream::new_tls(stream))
        }
    }
}

/// Creates a WebSocket handshake from a request and a stream,
/// upgrading the stream to TLS if required.
pub async fn client_async_tls<R, S>(
    request: R,
    stream: S,
) -> Result<(WebSocketStream<MaybeTlsStream<S>>, Response), Error>
where
    R: IntoClientRequest,
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    client_async_tls_with_config(request, stream, None, None).await
}

/// The same as `client_async_tls()` but the one can specify a websocket
/// configuration, and an optional connector.
pub async fn client_async_tls_with_config<R, S>(
    request: R,
    stream: S,
    connector: Option<TlsConnector>,
    config: Option<WebSocketConfig>,
) -> Result<(WebSocketStream<MaybeTlsStream<S>>, Response), Error>
where
    R: IntoClientRequest,
    S: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let request: Request = request.into_client_request()?;

    let domain = domain(&request)?;

    let mode = uri_mode(request.uri())?;

    let stream = wrap_stream(stream, domain, connector, mode).await?;
    client_async_with_config(request, stream, config).await
}

/// Type alias for a connected stream.
type ConnectStream = MaybeTlsStream<TcpStream>;

/// Connect to a given URL.
pub async fn connect_async<R>(
    request: R,
) -> Result<(WebSocketStream<ConnectStream>, Response), Error>
where
    R: IntoClientRequest,
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
    R: IntoClientRequest,
{
    connect_async_tls_with_config(request, config, disable_nagle, None).await
}

/// The same as `connect_async()` but the one can specify a websocket
/// configuration, a TLS connector, and whether to disable Nagle's algorithm.
/// `disable_nagle` specifies if the Nagle's algorithm must be disabled, i.e.
/// `set_nodelay(true)`. If you don't know what the Nagle's algorithm is, better
/// leave it to `false`.
pub async fn connect_async_tls_with_config<R>(
    request: R,
    config: Option<WebSocketConfig>,
    disable_nagle: bool,
    connector: Option<TlsConnector>,
) -> Result<(WebSocketStream<ConnectStream>, Response), Error>
where
    R: IntoClientRequest,
{
    let request: Request = request.into_client_request()?;

    // We don't check if it's an IPv6 address because `std` handles it internally.
    let domain = request
        .uri()
        .host()
        .ok_or(Error::Url(tungstenite::error::UrlError::NoHostName))?;
    let port = port(&request)?;

    let socket =
        TcpStream::connect_with_options((domain, port), TcpOpts::new().nodelay(disable_nagle))
            .await
            .map_err(Error::Io)?;
    client_async_tls_with_config(request, socket, connector, config).await
}

#[inline]
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

#[inline]
fn domain(request: &Request) -> Result<&str, Error> {
    request
        .uri()
        .host()
        .map(|host| {
            // If host is an IPv6 address, it might be surrounded by brackets. These
            // brackets are *not* part of a valid IP, so they must be stripped
            // out.
            //
            // The URI from the request is guaranteed to be valid, so we don't need a
            // separate check for the closing bracket.

            if host.starts_with('[') && host.ends_with(']') {
                &host[1..host.len() - 1]
            } else {
                host
            }
        })
        .ok_or(tungstenite::Error::Url(
            tungstenite::error::UrlError::NoHostName,
        ))
}
