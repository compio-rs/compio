use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::{AsyncRead, AsyncWrite, FutureExt};
use rustls::{
    ServerConfig, ServerConnection,
    server::{Acceptor, ClientHello},
};

use crate::TlsStream;

/// A lazy TLS acceptor that performs the initial handshake and allows access to
/// the [`ClientHello`] message before completing the handshake.
pub struct LazyConfigAcceptor<S>(futures_rustls::LazyConfigAcceptor<S>);

impl<S> LazyConfigAcceptor<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Create a new [`LazyConfigAcceptor`] with the given acceptor and stream.
    pub fn new(acceptor: Acceptor, s: S) -> Self {
        Self(futures_rustls::LazyConfigAcceptor::new(acceptor, s))
    }
}

impl<S> Future for LazyConfigAcceptor<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<StartHandshake<S>, io::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.poll_unpin(cx).map_ok(StartHandshake)
    }
}

/// A TLS acceptor that has completed the initial handshake and allows access to
/// the [`ClientHello`] message.
pub struct StartHandshake<S>(futures_rustls::StartHandshake<S>);

impl<S> StartHandshake<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Get the [`ClientHello`] message from the initial handshake.
    pub fn client_hello(&self) -> ClientHello<'_> {
        self.0.client_hello()
    }

    /// Complete the TLS handshake and return a [`TlsStream`] if successful.
    pub fn into_stream(
        self,
        config: Arc<ServerConfig>,
    ) -> impl Future<Output = io::Result<TlsStream<S>>> {
        self.into_stream_with(config, |_| ())
    }

    /// Complete the TLS handshake and return a [`TlsStream`] if successful.
    pub fn into_stream_with<F>(
        self,
        config: Arc<ServerConfig>,
        f: F,
    ) -> impl Future<Output = io::Result<TlsStream<S>>>
    where
        F: FnOnce(&mut ServerConnection),
    {
        self.0
            .into_stream_with(config, f)
            .map(|res| res.map(TlsStream::from))
    }
}
