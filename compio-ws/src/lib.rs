//! Async WebSocket support for compio.
//!
//! This library is an implementation of WebSocket handshakes and streams for
//! compio. It is based on the tungstenite crate which implements all required
//! WebSocket protocol logic. This crate brings compio support / compio
//! integration to it.
//!
//! Each WebSocket stream implements message reading and writing.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

pub mod stream;

#[cfg(feature = "rustls")]
pub mod rustls;

use std::io::ErrorKind;

use compio_buf::IntoInner;
use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
use tungstenite::{
    Error as WsError, HandshakeError, Message, WebSocket,
    client::IntoClientRequest,
    handshake::server::{Callback, NoCallback},
    protocol::CloseFrame,
};
pub use tungstenite::{
    Message as WebSocketMessage, error::Error as TungsteniteError, handshake::client::Response,
    protocol::WebSocketConfig,
};

#[cfg(feature = "rustls")]
pub use crate::rustls::{
    AutoStream, ConnectStream, Connector, client_async_tls, client_async_tls_with_config,
    client_async_tls_with_connector, client_async_tls_with_connector_and_config, connect_async,
    connect_async_with_config, connect_async_with_tls_connector,
    connect_async_with_tls_connector_and_config,
};

/// A WebSocket stream that works with compio.
pub struct WebSocketStream<S> {
    inner: WebSocket<SyncStream<S>>,
}

impl<S> WebSocketStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug,
{
    /// Send a message on the WebSocket stream.
    pub async fn send(&mut self, message: Message) -> Result<(), WsError> {
        // Send the message - this buffers it
        // Since CompioStream::flush() now returns Ok, this should succeed on first try
        self.inner.send(message)?;

        // flush the buffer to the network
        self.inner
            .get_mut()
            .flush_write_buf()
            .await
            .map_err(WsError::Io)?;

        Ok(())
    }

    /// Read a message from the WebSocket stream.
    pub async fn read(&mut self) -> Result<Message, WsError> {
        loop {
            match self.inner.read() {
                Ok(msg) => {
                    // Always try to flush after read (close frames need this)
                    let _ = self.inner.get_mut().flush_write_buf().await;
                    return Ok(msg);
                }
                Err(WsError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
                    // Need more data - fill the read buffer
                    self.inner
                        .get_mut()
                        .fill_read_buf()
                        .await
                        .map_err(WsError::Io)?;
                    // Retry the read
                    continue;
                }
                Err(e) => {
                    // Always try to flush on error (close frames)
                    let _ = self.inner.get_mut().flush_write_buf().await;
                    return Err(e);
                }
            }
        }
    }

    /// Close the WebSocket connection.
    pub async fn close(&mut self, close_frame: Option<CloseFrame>) -> Result<(), WsError> {
        loop {
            match self.inner.close(close_frame.clone()) {
                Ok(()) => return Ok(()),
                Err(WsError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
                    let sync_stream = self.inner.get_mut();

                    let flushed = sync_stream.flush_write_buf().await.map_err(WsError::Io)?;

                    if flushed == 0 {
                        sync_stream.fill_read_buf().await.map_err(WsError::Io)?;
                    }
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Get a reference to the underlying stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref().get_ref()
    }

    /// Get a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut().get_mut()
    }

    /// Get the inner WebSocket.
    #[deprecated = "Use IntoInner trait instead. This method will be removed in a future release."]
    pub fn get_inner(self) -> WebSocket<SyncStream<S>> {
        self.inner
    }
}

impl<S> IntoInner for WebSocketStream<S> {
    type Inner = WebSocket<SyncStream<S>>;

    fn into_inner(self) -> Self::Inner {
        self.inner
    }
}

/// Accepts a new WebSocket connection with the provided stream.
///
/// This function will internally call `server::accept` to create a
/// handshake representation and returns a future representing the
/// resolution of the WebSocket handshake. The returned future will resolve
/// to either `WebSocketStream<S>` or `Error` depending if it's successful
/// or not.
///
/// This is typically used after a socket has been accepted from a
/// `TcpListener`. That socket is then passed to this function to perform
/// the server half of accepting a client's websocket connection.
pub async fn accept_async<S>(stream: S) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug,
{
    accept_hdr_async(stream, NoCallback).await
}

/// The same as `accept_async()` but the one can specify a websocket
/// configuration. Please refer to `accept_async()` for more details.
pub async fn accept_async_with_config<S>(
    stream: S,
    config: Option<WebSocketConfig>,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug,
{
    accept_hdr_with_config_async(stream, NoCallback, config).await
}
/// Accepts a new WebSocket connection with the provided stream.
///
/// This function does the same as `accept_async()` but accepts an extra
/// callback for header processing. The callback receives headers of the
/// incoming requests and is able to add extra headers to the reply.
pub async fn accept_hdr_async<S, C>(stream: S, callback: C) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug,
    C: Callback,
{
    accept_hdr_with_config_async(stream, callback, None).await
}

/// The same as `accept_hdr_async()` but the one can specify a websocket
/// configuration. Please refer to `accept_hdr_async()` for more details.
pub async fn accept_hdr_with_config_async<S, C>(
    stream: S,
    callback: C,
    config: Option<WebSocketConfig>,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug,
    C: Callback,
{
    let sync_stream = SyncStream::with_capacity(128 * 1024, stream);
    let mut handshake_result = tungstenite::accept_hdr_with_config(sync_stream, callback, config);

    loop {
        match handshake_result {
            Ok(mut websocket) => {
                websocket
                    .get_mut()
                    .flush_write_buf()
                    .await
                    .map_err(WsError::Io)?;
                return Ok(WebSocketStream { inner: websocket });
            }
            Err(HandshakeError::Interrupted(mut mid_handshake)) => {
                let sync_stream = mid_handshake.get_mut().get_mut();

                sync_stream.flush_write_buf().await.map_err(WsError::Io)?;

                sync_stream.fill_read_buf().await.map_err(WsError::Io)?;

                handshake_result = mid_handshake.handshake();
            }
            Err(HandshakeError::Failure(error)) => {
                return Err(error);
            }
        }
    }
}

/// Creates a WebSocket handshake from a request and a stream.
///
/// For convenience, the user may call this with a url string, a URL,
/// or a `Request`. Calling with `Request` allows the user to add
/// a WebSocket protocol or other custom headers.
///
/// Internally, this creates a handshake representation and returns
/// a future representing the resolution of the WebSocket handshake. The
/// returned future will resolve to either `WebSocketStream<S>` or `Error`
/// depending on whether the handshake is successful.
///
/// This is typically used for clients who have already established, for
/// example, a TCP connection to the remote server.
pub async fn client_async<R, S>(
    request: R,
    stream: S,
) -> Result<(WebSocketStream<S>, tungstenite::handshake::client::Response), WsError>
where
    R: IntoClientRequest,
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug,
{
    client_async_with_config(request, stream, None).await
}

/// The same as `client_async()` but the one can specify a websocket
/// configuration. Please refer to `client_async()` for more details.
pub async fn client_async_with_config<R, S>(
    request: R,
    stream: S,
    config: Option<WebSocketConfig>,
) -> Result<(WebSocketStream<S>, tungstenite::handshake::client::Response), WsError>
where
    R: IntoClientRequest,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let sync_stream = SyncStream::with_capacity(128 * 1024, stream);
    let mut handshake_result =
        tungstenite::client::client_with_config(request, sync_stream, config);

    loop {
        match handshake_result {
            Ok((mut websocket, response)) => {
                // Ensure any remaining data is flushed
                websocket
                    .get_mut()
                    .flush_write_buf()
                    .await
                    .map_err(WsError::Io)?;
                return Ok((WebSocketStream { inner: websocket }, response));
            }
            Err(HandshakeError::Interrupted(mut mid_handshake)) => {
                let sync_stream = mid_handshake.get_mut().get_mut();

                // For handshake: always try both operations
                sync_stream.flush_write_buf().await.map_err(WsError::Io)?;

                sync_stream.fill_read_buf().await.map_err(WsError::Io)?;

                handshake_result = mid_handshake.handshake();
            }
            Err(HandshakeError::Failure(error)) => {
                return Err(error);
            }
        }
    }
}

#[inline]
#[allow(clippy::result_large_err)]
#[cfg(feature = "rustls")]
pub(crate) fn domain(
    request: &tungstenite::handshake::client::Request,
) -> Result<String, tungstenite::Error> {
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
            let host = if host.starts_with('[') {
                &host[1..host.len() - 1]
            } else {
                host
            };

            host.to_owned()
        })
        .ok_or(tungstenite::Error::Url(
            tungstenite::error::UrlError::NoHostName,
        ))
}
