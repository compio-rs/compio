//! WebSocket support based on [`tungstenite`].
//!
//! This library is an implementation of WebSocket handshakes and streams for
//! compio. It is based on the tungstenite crate which implements all required
//! WebSocket protocol logic. This crate brings compio support / compio
//! integration to it.
//!
//! Each WebSocket stream implements message reading and writing.
//!
//! [`tungstenite`]: https://docs.rs/tungstenite

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

use std::io::ErrorKind;

use compio_buf::IntoInner;
use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
use tungstenite::{
    Error as WsError, HandshakeError, Message, WebSocket,
    client::IntoClientRequest,
    handshake::server::{Callback, NoCallback},
    protocol::{CloseFrame, Role, WebSocketConfig},
};

mod tls;
#[cfg(feature = "io-compat")]
pub use compat::CompatWebSocketStream;
pub use tls::*;
pub use tungstenite;
#[cfg(feature = "io-compat")]
mod compat;

/// Configuration for compio-ws.
///
/// ## API Interface
///
/// `_with_config` functions in this crate accept `impl Into<Config>`, so
/// following are all valid:
/// - [`Config`]
/// - [`WebSocketConfig`] (use custom WebSocket config with default remaining
///   settings)
/// - [`None`] (use default value)
pub struct Config {
    /// WebSocket configuration from tungstenite.
    websocket: Option<WebSocketConfig>,

    /// Base buffer size
    buffer_size_base: usize,

    /// Maximum buffer size
    buffer_size_limit: usize,

    /// Disable Nagle's algorithm. This only affects
    /// [`connect_async_with_config()`] and [`connect_async_tls_with_config()`].
    disable_nagle: bool,
}

impl Config {
    // 128 KiB, see <https://github.com/compio-rs/compio/pull/532>.
    const DEFAULT_BUF_SIZE: usize = 128 * 1024;
    // 64 MiB, the same as [`SyncStream`].
    const DEFAULT_MAX_BUFFER: usize = 64 * 1024 * 1024;

    /// Creates a new `Config` with default settings.
    pub fn new() -> Self {
        Self {
            websocket: None,
            buffer_size_base: Self::DEFAULT_BUF_SIZE,
            buffer_size_limit: Self::DEFAULT_MAX_BUFFER,
            disable_nagle: false,
        }
    }

    /// Get the WebSocket configuration.
    pub fn websocket_config(&self) -> Option<&WebSocketConfig> {
        self.websocket.as_ref()
    }

    /// Get the base buffer size.
    pub fn buffer_size_base(&self) -> usize {
        self.buffer_size_base
    }

    /// Get the maximum buffer size.
    pub fn buffer_size_limit(&self) -> usize {
        self.buffer_size_limit
    }

    /// Set custom base buffer size.
    ///
    /// Default to 128 KiB.
    pub fn with_buffer_size_base(mut self, size: usize) -> Self {
        self.buffer_size_base = size;
        self
    }

    /// Set custom maximum buffer size.
    ///
    /// Default to 64 MiB.
    pub fn with_buffer_size_limit(mut self, size: usize) -> Self {
        self.buffer_size_limit = size;
        self
    }

    /// Set custom buffer sizes.
    ///
    /// Default to 128 KiB for base and 64 MiB for limit.
    pub fn with_buffer_sizes(mut self, base: usize, limit: usize) -> Self {
        self.buffer_size_base = base;
        self.buffer_size_limit = limit;
        self
    }

    /// Disable Nagle's algorithm, i.e. `set_nodelay(true)`.
    ///
    /// Default to `false`. If you don't know what the Nagle's algorithm is,
    /// better leave it to `false`.
    pub fn disable_nagle(mut self, disable: bool) -> Self {
        self.disable_nagle = disable;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl From<WebSocketConfig> for Config {
    fn from(config: WebSocketConfig) -> Self {
        Self {
            websocket: Some(config),
            ..Default::default()
        }
    }
}

impl From<Option<WebSocketConfig>> for Config {
    fn from(config: Option<WebSocketConfig>) -> Self {
        Self {
            websocket: config,
            ..Default::default()
        }
    }
}

/// A WebSocket stream that works with compio.
#[derive(Debug)]
pub struct WebSocketStream<S> {
    inner: WebSocket<SyncStream<S>>,
}

impl<S> WebSocketStream<S> {
    /// Get a reference to the underlying stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref().get_ref()
    }

    /// Get a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut().get_mut()
    }
}

impl<S> WebSocketStream<S>
where
    S: AsyncRead + AsyncWrite,
{
    /// Convert a raw socket into a [`WebSocketStream`] without performing a
    /// handshake.
    ///
    /// `disable_nagle` will be ignored since the socket is already connected
    /// and the user can set `nodelay` on the socket directly before calling
    /// this function if needed.
    pub async fn from_raw_socket(stream: S, role: Role, config: impl Into<Config>) -> Self {
        let config = config.into();
        let sync_stream =
            SyncStream::with_limits(config.buffer_size_base, config.buffer_size_limit, stream);

        WebSocketStream {
            inner: WebSocket::from_raw_socket(sync_stream, role, config.websocket),
        }
    }

    /// Convert a raw socket into a [`WebSocketStream`] without performing a
    /// handshake.
    ///
    /// `disable_nagle` will be ignored since the socket is already connected
    /// and the user can set `nodelay` on the socket directly before calling
    /// this function if needed.
    pub async fn from_partially_read(
        stream: S,
        part: Vec<u8>,
        role: Role,
        config: impl Into<Config>,
    ) -> Self {
        let config = config.into();
        let sync_stream =
            SyncStream::with_limits(config.buffer_size_base, config.buffer_size_limit, stream);

        WebSocketStream {
            inner: WebSocket::from_partially_read(sync_stream, part, role, config.websocket),
        }
    }

    /// Send a message on the WebSocket stream.
    pub async fn send(&mut self, message: Message) -> Result<(), WsError> {
        match self.inner.write(message) {
            Ok(()) => {}
            Err(WsError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        // Need to flush the write buffer before we can send the message
        self.flush().await
    }

    /// Read a message from the WebSocket stream.
    pub async fn read(&mut self) -> Result<Message, WsError> {
        loop {
            match self.inner.read() {
                Ok(msg) => {
                    self.flush().await?;
                    return Ok(msg);
                }
                Err(WsError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
                    // Need more data - fill the read buffer
                    self.fill_read_buf().await?;
                }
                Err(e) => {
                    let _ = self.flush().await;
                    return Err(e);
                }
            }
        }
    }

    /// Flush the WebSocket stream.
    pub async fn flush(&mut self) -> Result<(), WsError> {
        loop {
            match self.inner.flush() {
                Ok(()) => break,
                Err(WsError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
                    self.flush_write_buf().await?;
                }
                Err(WsError::ConnectionClosed) => break,
                Err(e) => return Err(e),
            }
        }
        self.flush_write_buf().await?;
        Ok(())
    }

    /// Close the WebSocket connection.
    pub async fn close(&mut self, close_frame: Option<CloseFrame>) -> Result<(), WsError> {
        loop {
            match self.inner.close(close_frame.clone()) {
                Ok(()) => break,
                Err(WsError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
                    let flushed = self.flush_write_buf().await?;
                    if flushed == 0 {
                        self.fill_read_buf().await?;
                    }
                }
                Err(WsError::ConnectionClosed) => break,
                Err(e) => return Err(e),
            }
        }
        self.flush().await
    }

    pub(crate) async fn flush_write_buf(&mut self) -> Result<usize, WsError> {
        self.inner
            .get_mut()
            .flush_write_buf()
            .await
            .map_err(WsError::Io)
    }

    pub(crate) async fn fill_read_buf(&mut self) -> Result<usize, WsError> {
        self.inner
            .get_mut()
            .fill_read_buf()
            .await
            .map_err(WsError::Io)
    }

    /// Convert this stream into a [`futures_util`] compatible stream.
    #[cfg(feature = "io-compat")]
    pub fn into_compat(self) -> CompatWebSocketStream<S>
    // Ensure internal mutability of the stream.
    where
        for<'a> &'a S: AsyncRead + AsyncWrite,
    {
        CompatWebSocketStream::new(self)
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
/// This function will internally create a handshake representation and returns
/// a future representing the resolution of the WebSocket handshake. The
/// returned future will resolve to either [`WebSocketStream<S>`] or [`WsError`]
/// depending on if it's successful or not.
///
/// This is typically used after a socket has been accepted from a
/// `TcpListener`. That socket is then passed to this function to perform
/// the server half of accepting a client's websocket connection.
pub async fn accept_async<S>(stream: S) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite,
{
    accept_hdr_async(stream, NoCallback).await
}

/// Similar to [`accept_async()`] but user can specify a [`Config`].
pub async fn accept_async_with_config<S>(
    stream: S,
    config: impl Into<Config>,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite,
{
    accept_hdr_with_config_async(stream, NoCallback, config).await
}
/// Accepts a new WebSocket connection with the provided stream.
///
/// This function does the same as [`accept_async()`] but accepts an extra
/// callback for header processing. The callback receives headers of the
/// incoming requests and is able to add extra headers to the reply.
pub async fn accept_hdr_async<S, C>(stream: S, callback: C) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite,
    C: Callback,
{
    accept_hdr_with_config_async(stream, callback, None).await
}

/// Similar to [`accept_hdr_async()`] but user can specify a [`Config`].
pub async fn accept_hdr_with_config_async<S, C>(
    stream: S,
    callback: C,
    config: impl Into<Config>,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsyncRead + AsyncWrite,
    C: Callback,
{
    let config = config.into();
    let sync_stream =
        SyncStream::with_limits(config.buffer_size_base, config.buffer_size_limit, stream);
    let mut handshake_result =
        tungstenite::accept_hdr_with_config(sync_stream, callback, config.websocket);

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
/// returned future will resolve to either [`WebSocketStream<S>`] or [`WsError`]
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
    S: AsyncRead + AsyncWrite,
{
    client_async_with_config(request, stream, None).await
}

/// Similar to [`client_async()`] but user can specify a [`Config`].
pub async fn client_async_with_config<R, S>(
    request: R,
    stream: S,
    config: impl Into<Config>,
) -> Result<(WebSocketStream<S>, tungstenite::handshake::client::Response), WsError>
where
    R: IntoClientRequest,
    S: AsyncRead + AsyncWrite,
{
    let config = config.into();
    let sync_stream =
        SyncStream::with_limits(config.buffer_size_base, config.buffer_size_limit, stream);
    let mut handshake_result =
        tungstenite::client::client_with_config(request, sync_stream, config.websocket);

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
