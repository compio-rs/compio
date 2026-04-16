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
#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::IntoInner;
use compio_io::{AsyncRead, AsyncWrite, compat::AsyncStream, util::Splittable};
use futures_util::{AsyncWriteExt, Sink, SinkExt, Stream, StreamExt, stream::FusedStream};
use pin_project_lite::pin_project;
use tungstenite::{
    Error as WsError, Message,
    client::IntoClientRequest,
    handshake::server::{Callback, NoCallback},
    protocol::{CloseFrame, Role, WebSocketConfig},
};

mod tls;
pub use tls::*;
pub use tungstenite;

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

pin_project! {
    /// A WebSocket stream that works with compio.
    #[derive(Debug)]
    pub struct WebSocketStream<S> {
        #[pin]
        inner: async_tungstenite::WebSocketStream<S>,
    }
}

/// A WebSocket stream with a plain underlying stream.
pub type WebSocketStreamPlain<S> = WebSocketStream<Pin<Box<AsyncStream<S>>>>;

impl<S> WebSocketStream<S>
where
    S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
{
    /// Get a reference to the underlying stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
    }

    /// Get a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut()
    }

    /// Convert a raw socket into a [`WebSocketStream`] without performing a
    /// handshake.
    ///
    /// `disable_nagle` will be ignored since the socket is already connected
    /// and the user can set `nodelay` on the socket directly before calling
    /// this function if needed.
    pub async fn from_raw_socket(stream: S, role: Role, config: impl Into<Config>) -> Self {
        let config = config.into();

        WebSocketStream {
            inner: async_tungstenite::WebSocketStream::from_raw_socket(
                stream,
                role,
                config.websocket,
            )
            .await,
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

        WebSocketStream {
            inner: async_tungstenite::WebSocketStream::from_partially_read(
                stream,
                part,
                role,
                config.websocket,
            )
            .await,
        }
    }

    /// Send a message on the WebSocket stream.
    pub async fn send(&mut self, message: Message) -> Result<(), WsError> {
        self.inner.send(message).await
    }

    /// Read a message from the WebSocket stream.
    pub async fn read(&mut self) -> Result<Message, WsError> {
        let msg = self
            .inner
            .next()
            .await
            .unwrap_or_else(|| Err(WsError::ConnectionClosed))?;
        self.flush().await?;
        Ok(msg)
    }

    /// Flush the WebSocket stream.
    pub async fn flush(&mut self) -> Result<(), WsError> {
        self.inner.flush().await?;
        self.inner.get_mut().flush().await?;
        Ok(())
    }

    /// Close the WebSocket connection.
    pub async fn close(&mut self, close_frame: Option<CloseFrame>) -> Result<(), WsError> {
        self.inner.close(close_frame).await
    }
}

impl<S> IntoInner for WebSocketStream<S> {
    type Inner = S;

    fn into_inner(self) -> Self::Inner {
        self.inner.into_inner()
    }
}

impl<S> Sink<Message> for WebSocketStream<S>
where
    S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
{
    type Error = WsError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), WsError>> {
        self.project().inner.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.project().inner.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_close(cx)
    }
}

impl<S> Stream for WebSocketStream<S>
where
    S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
{
    type Item = Result<Message, WsError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

impl<S> FusedStream for WebSocketStream<S>
where
    S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
{
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
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
pub async fn accept_async<S>(stream: S) -> Result<WebSocketStreamPlain<S>, WsError>
where
    S: Splittable + 'static,
    <S as Splittable>::ReadHalf: AsyncRead + Unpin,
    <S as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
    accept_hdr_async(stream, NoCallback).await
}

/// Similar to [`accept_async()`] but user can specify a [`Config`].
pub async fn accept_async_with_config<S>(
    stream: S,
    config: impl Into<Config>,
) -> Result<WebSocketStreamPlain<S>, WsError>
where
    S: Splittable + 'static,
    <S as Splittable>::ReadHalf: AsyncRead + Unpin,
    <S as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
    accept_hdr_with_config_async(stream, NoCallback, config).await
}
/// Accepts a new WebSocket connection with the provided stream.
///
/// This function does the same as [`accept_async()`] but accepts an extra
/// callback for header processing. The callback receives headers of the
/// incoming requests and is able to add extra headers to the reply.
pub async fn accept_hdr_async<S, C>(
    stream: S,
    callback: C,
) -> Result<WebSocketStreamPlain<S>, WsError>
where
    S: Splittable + 'static,
    C: Callback + Unpin,
    <S as Splittable>::ReadHalf: AsyncRead + Unpin,
    <S as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
    accept_hdr_with_config_async(stream, callback, None).await
}

/// Similar to [`accept_hdr_async()`] but user can specify a [`Config`].
pub async fn accept_hdr_with_config_async<S, C>(
    stream: S,
    callback: C,
    config: impl Into<Config>,
) -> Result<WebSocketStreamPlain<S>, WsError>
where
    S: Splittable + 'static,
    C: Callback + Unpin,
    <S as Splittable>::ReadHalf: AsyncRead + Unpin,
    <S as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
    let config = config.into();
    let inner = async_tungstenite::accept_hdr_async_with_config(
        Box::pin(AsyncStream::new(stream)),
        callback,
        config.websocket,
    )
    .await?;
    Ok(WebSocketStream { inner })
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
) -> Result<
    (
        WebSocketStreamPlain<S>,
        tungstenite::handshake::client::Response,
    ),
    WsError,
>
where
    R: IntoClientRequest + Unpin,
    S: Splittable + 'static,
    <S as Splittable>::ReadHalf: AsyncRead + Unpin,
    <S as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
    client_async_with_config(request, stream, None).await
}

/// Similar to [`client_async()`] but user can specify a [`Config`].
pub async fn client_async_with_config<R, S>(
    request: R,
    stream: S,
    config: impl Into<Config>,
) -> Result<
    (
        WebSocketStreamPlain<S>,
        tungstenite::handshake::client::Response,
    ),
    WsError,
>
where
    R: IntoClientRequest + Unpin,
    S: Splittable + 'static,
    <S as Splittable>::ReadHalf: AsyncRead + Unpin,
    <S as Splittable>::WriteHalf: AsyncWrite + Unpin,
{
    client_async_with_config_compat(request, Box::pin(AsyncStream::new(stream)), config).await
}

pub(crate) async fn client_async_with_config_compat<R, S>(
    request: R,
    stream: S,
    config: impl Into<Config>,
) -> Result<(WebSocketStream<S>, tungstenite::handshake::client::Response), WsError>
where
    R: IntoClientRequest + Unpin,
    S: futures_util::AsyncRead + futures_util::AsyncWrite + Unpin,
{
    let config = config.into();
    let (inner, response) =
        async_tungstenite::client_async_with_config(request, stream, config.websocket).await?;
    Ok((WebSocketStream { inner }, response))
}
