//! TLS support for HTTP/2 connections.
//!
//! Provides helper functions to establish TLS-secured HTTP/2 connections with
//! ALPN negotiation. The resulting streams are wrapped in
//! [`Split`](compio_io::util::split::Split) so they
//! can be passed directly to [`client::handshake`](crate::client::handshake)
//! or [`server::handshake`](crate::server::handshake).

use std::io;

use compio_io::{AsyncRead, AsyncWrite, util::split::Split};

pub use compio_tls::{TlsAcceptor, TlsConnector, TlsStream};
#[cfg(feature = "native-tls")]
pub use compio_tls::native_tls;
#[cfg(feature = "rustls")]
pub use compio_tls::rustls;

/// Perform a TLS client handshake with ALPN `h2` validation.
///
/// Connects via the provided [`TlsConnector`], checks that the server
/// negotiated the `h2` ALPN protocol, and wraps the resulting stream in
/// [`Split`] so it satisfies the [`Splittable`](compio_io::util::Splittable)
/// bound required by [`client::handshake`](crate::client::handshake).
pub async fn connect<S>(
    connector: &TlsConnector,
    domain: &str,
    stream: S,
) -> io::Result<Split<TlsStream<S>>>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    let tls_stream = connector.connect(domain, stream).await?;

    // Verify the server negotiated HTTP/2 via ALPN.
    match tls_stream.negotiated_alpn() {
        Some(ref proto) if proto.as_ref() == b"h2" => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "server did not negotiate h2 ALPN",
            ));
        }
    }

    Ok(Split::new(tls_stream))
}

/// Perform a TLS server handshake.
///
/// Accepts via the provided [`TlsAcceptor`] and wraps the resulting stream
/// in [`Split`] so it satisfies the [`Splittable`](compio_io::util::Splittable)
/// bound required by [`server::handshake`](crate::server::handshake).
///
/// ALPN validation is not enforced on the server side — the server advertises
/// `h2` but the final protocol choice is made by the client.
pub async fn accept<S>(
    acceptor: &TlsAcceptor,
    stream: S,
) -> io::Result<Split<TlsStream<S>>>
where
    S: AsyncRead + AsyncWrite + Unpin + 'static,
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    let tls_stream = acceptor.accept(stream).await?;
    Ok(Split::new(tls_stream))
}
