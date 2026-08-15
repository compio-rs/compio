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
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::IntoInner;
use compio_driver::AsFd;
use compio_net::{TcpStream, UnixStream};
use compio_runtime::fd::PollFd;
use compio_tls::{MaybeTlsStream, TlsStream};
use futures_util::{Sink, SinkExt, Stream, StreamExt, stream::FusedStream};
use pin_project_lite::pin_project;
use socket2::Socket;
use tungstenite::{
    Error as WsError, Message,
    client::IntoClientRequest,
    handshake::server::{Callback, NoCallback},
    protocol::{CloseFrame, Role, WebSocketConfig},
};

#[cfg(feature = "connect")]
mod tls;
#[cfg(feature = "connect")]
pub use tls::*;
pub use tungstenite;

type MaybePollStream<S> = MaybeTlsStream<PollFd<S>>;

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

    /// Disable Nagle's algorithm. This only affects
    /// [`connect_async_with_config()`] and [`connect_async_tls_with_config()`].
    disable_nagle: bool,
}

impl Config {
    /// Creates a new `Config` with default settings.
    pub fn new() -> Self {
        Self {
            websocket: None,
            disable_nagle: false,
        }
    }

    /// Get the WebSocket configuration.
    pub fn websocket_config(&self) -> Option<&WebSocketConfig> {
        self.websocket.as_ref()
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

mod private {
    use super::*;

    pub trait Sealed<S>
    where
        S: AsFd,
    {
    }

    impl<S: AsFd> Sealed<S> for PollFd<S> {}
    impl<S: AsFd> Sealed<S> for MaybePollStream<S> {}
    impl<S: AsFd> Sealed<S> for TlsStream<PollFd<S>> {}
    impl Sealed<Socket> for TcpStream {}
    impl Sealed<Socket> for UnixStream {}
}

/// Convert a stream into a [`MaybeTlsStream`].
pub trait IntoMaybeTlsStream<S>: private::Sealed<S>
where
    S: AsFd,
{
    /// Convert this stream into a [`MaybeTlsStream`].
    fn into_maybe_tls_stream(self) -> io::Result<MaybePollStream<S>>;
}

impl<S: AsFd> IntoMaybeTlsStream<S> for PollFd<S> {
    fn into_maybe_tls_stream(self) -> io::Result<MaybePollStream<S>> {
        Ok(MaybeTlsStream::new_plain(self))
    }
}

impl<S: AsFd> IntoMaybeTlsStream<S> for MaybePollStream<S> {
    fn into_maybe_tls_stream(self) -> io::Result<MaybePollStream<S>> {
        Ok(self)
    }
}

impl<S: AsFd> IntoMaybeTlsStream<S> for TlsStream<PollFd<S>> {
    fn into_maybe_tls_stream(self) -> io::Result<MaybePollStream<S>> {
        Ok(MaybeTlsStream::new_tls(self))
    }
}

impl IntoMaybeTlsStream<Socket> for TcpStream {
    fn into_maybe_tls_stream(self) -> io::Result<MaybePollStream<Socket>> {
        Ok(MaybeTlsStream::new_plain(self.into_poll_fd()?))
    }
}

impl IntoMaybeTlsStream<Socket> for UnixStream {
    fn into_maybe_tls_stream(self) -> io::Result<MaybePollStream<Socket>> {
        Ok(MaybeTlsStream::new_plain(self.into_poll_fd()?))
    }
}

pin_project! {
    /// A WebSocket stream that works with compio.
    #[derive(Debug)]
    pub struct WebSocketStream<S: AsFd> {
        #[pin]
        inner: async_tungstenite::WebSocketStream<MaybePollStream<S>>,
        next_item: Option<Option<Result<Message, WsError>>>,
    }
}

impl<S: AsFd + 'static> WebSocketStream<S> {
    /// Get a reference to the underlying stream.
    pub fn get_ref(&self) -> &MaybePollStream<S> {
        self.inner.get_ref()
    }

    /// Get a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut MaybePollStream<S> {
        self.inner.get_mut()
    }

    /// Convert a raw socket into a [`WebSocketStream`] without performing a
    /// handshake.
    ///
    /// `disable_nagle` will be ignored since the socket is already connected
    /// and the user can set `nodelay` on the socket directly before calling
    /// this function if needed.
    pub async fn from_raw_socket<T: IntoMaybeTlsStream<S>>(
        stream: T,
        role: Role,
        config: impl Into<Config>,
    ) -> io::Result<Self> {
        let config = config.into();

        Ok(Self::from_inner(
            async_tungstenite::WebSocketStream::from_raw_socket(
                stream.into_maybe_tls_stream()?,
                role,
                config.websocket,
            )
            .await,
        ))
    }

    /// Convert a raw socket into a [`WebSocketStream`] without performing a
    /// handshake.
    ///
    /// `disable_nagle` will be ignored since the socket is already connected
    /// and the user can set `nodelay` on the socket directly before calling
    /// this function if needed.
    pub async fn from_partially_read<T: IntoMaybeTlsStream<S>>(
        stream: T,
        part: Vec<u8>,
        role: Role,
        config: impl Into<Config>,
    ) -> io::Result<Self> {
        let config = config.into();

        Ok(Self::from_inner(
            async_tungstenite::WebSocketStream::from_partially_read(
                stream.into_maybe_tls_stream()?,
                part,
                role,
                config.websocket,
            )
            .await,
        ))
    }

    fn from_inner(inner: async_tungstenite::WebSocketStream<MaybePollStream<S>>) -> Self {
        WebSocketStream {
            inner,
            next_item: None,
        }
    }

    /// Send a message on the WebSocket stream.
    pub async fn send(&mut self, message: Message) -> Result<(), WsError> {
        SinkExt::send(self, message).await
    }

    /// Read a message from the WebSocket stream.
    pub async fn read(&mut self) -> Result<Message, WsError> {
        self.next()
            .await
            .unwrap_or_else(|| Err(WsError::ConnectionClosed))
    }

    /// Flush the WebSocket stream.
    pub async fn flush(&mut self) -> Result<(), WsError> {
        SinkExt::flush(self).await
    }

    /// Close the WebSocket connection.
    pub async fn close(&mut self, close_frame: Option<CloseFrame>) -> Result<(), WsError> {
        self.send(Message::Close(close_frame)).await
    }
}

impl<S: AsFd> IntoInner for WebSocketStream<S> {
    type Inner = MaybePollStream<S>;

    fn into_inner(self) -> Self::Inner {
        self.inner.into_inner()
    }
}

impl<S: AsFd + 'static> Sink<Message> for WebSocketStream<S> {
    type Error = WsError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), WsError>> {
        self.project().inner.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.project().inner.start_send(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.as_mut().project().inner.poll_flush(cx))?;
        ready!(futures_util::AsyncWrite::poll_flush(
            Pin::new(self.project().inner.get_mut().get_mut()),
            cx
        ))?;
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_close(cx)
    }
}

impl<S: AsFd + 'static> Stream for WebSocketStream<S> {
    type Item = Result<Message, WsError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            if this.next_item.is_some() {
                ready!(this.inner.as_mut().poll_flush(cx))?;
                ready!(futures_util::AsyncWrite::poll_flush(
                    Pin::new(this.inner.get_mut().get_mut()),
                    cx
                ))?;
                break Poll::Ready(this.next_item.take().expect("next_item should be Some"));
            } else {
                let item = ready!(this.inner.as_mut().poll_next(cx));
                *this.next_item = Some(item);
            }
        }
    }
}

impl<S: AsFd + 'static> FusedStream for WebSocketStream<S> {
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
pub async fn accept_async<S, T>(stream: T) -> Result<WebSocketStream<S>, WsError>
where
    S: AsFd + 'static,
    T: IntoMaybeTlsStream<S>,
{
    accept_hdr_async(stream, NoCallback).await
}

/// Similar to [`accept_async()`] but user can specify a [`Config`].
pub async fn accept_async_with_config<S, T>(
    stream: T,
    config: impl Into<Config>,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsFd + 'static,
    T: IntoMaybeTlsStream<S>,
{
    accept_hdr_with_config_async(stream, NoCallback, config).await
}

/// Accepts a new WebSocket connection with the provided stream.
///
/// This function does the same as [`accept_async()`] but accepts an extra
/// callback for header processing. The callback receives headers of the
/// incoming requests and is able to add extra headers to the reply.
pub async fn accept_hdr_async<S, T, C>(
    stream: T,
    callback: C,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsFd + 'static,
    T: IntoMaybeTlsStream<S>,
    C: Callback + Unpin,
{
    accept_hdr_with_config_async(stream, callback, None).await
}

/// Similar to [`accept_hdr_async()`] but user can specify a [`Config`].
pub async fn accept_hdr_with_config_async<S, T, C>(
    stream: T,
    callback: C,
    config: impl Into<Config>,
) -> Result<WebSocketStream<S>, WsError>
where
    S: AsFd + 'static,
    T: IntoMaybeTlsStream<S>,
    C: Callback + Unpin,
{
    let config = config.into();
    let inner = async_tungstenite::accept_hdr_async_with_config(
        stream.into_maybe_tls_stream()?,
        callback,
        config.websocket,
    )
    .await?;
    Ok(WebSocketStream::from_inner(inner))
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
pub async fn client_async<R, S, T>(
    request: R,
    stream: T,
) -> Result<(WebSocketStream<S>, tungstenite::handshake::client::Response), WsError>
where
    R: IntoClientRequest + Unpin,
    S: AsFd + 'static,
    T: IntoMaybeTlsStream<S>,
{
    client_async_with_config(request, stream, None).await
}

/// Similar to [`client_async()`] but user can specify a [`Config`].
pub async fn client_async_with_config<R, S, T>(
    request: R,
    stream: T,
    config: impl Into<Config>,
) -> Result<(WebSocketStream<S>, tungstenite::handshake::client::Response), WsError>
where
    R: IntoClientRequest + Unpin,
    S: AsFd + 'static,
    T: IntoMaybeTlsStream<S>,
{
    let config = config.into();
    let (inner, response) = async_tungstenite::client_async_with_config(
        request,
        stream.into_maybe_tls_stream()?,
        config.websocket,
    )
    .await?;
    Ok((WebSocketStream::from_inner(inner), response))
}
