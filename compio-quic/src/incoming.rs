use std::{
    future::{Future, IntoFuture},
    net::{IpAddr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::FutureExt;
use quinn_proto::ServerConfig;
use thiserror::Error;

use crate::{Connecting, Connection, ConnectionError, EndpointRef};

#[derive(Debug)]
pub(crate) struct IncomingInner {
    pub(crate) incoming: quinn_proto::Incoming,
    pub(crate) endpoint: EndpointRef,
}

/// An incoming connection for which the server has not yet begun its part
/// of the handshake.
#[derive(Debug)]
pub struct Incoming(Option<IncomingInner>);

impl Incoming {
    pub(crate) fn new(incoming: quinn_proto::Incoming, endpoint: EndpointRef) -> Self {
        Self(Some(IncomingInner { incoming, endpoint }))
    }

    /// Attempt to accept this incoming connection (an error may still
    /// occur).
    pub fn accept(mut self) -> Result<Connecting, ConnectionError> {
        let inner = self.0.take().unwrap();
        Ok(inner.endpoint.accept(inner.incoming, None)?)
    }

    /// Accept this incoming connection using a custom configuration.
    ///
    /// See [`accept()`] for more details.
    ///
    /// [`accept()`]: Incoming::accept
    pub fn accept_with(
        mut self,
        server_config: ServerConfig,
    ) -> Result<Connecting, ConnectionError> {
        let inner = self.0.take().unwrap();
        Ok(inner.endpoint.accept(inner.incoming, Some(server_config))?)
    }

    /// Reject this incoming connection attempt.
    pub fn refuse(mut self) {
        let inner = self.0.take().unwrap();
        inner.endpoint.refuse(inner.incoming);
    }

    /// Respond with a retry packet, requiring the client to retry with
    /// address validation.
    ///
    /// Errors if `remote_address_validated()` is true.
    #[allow(clippy::result_large_err)]
    pub fn retry(mut self) -> Result<(), RetryError> {
        let inner = self.0.take().unwrap();
        inner
            .endpoint
            .retry(inner.incoming)
            .map_err(|e| RetryError(Self::new(e.into_incoming(), inner.endpoint)))
    }

    /// Ignore this incoming connection attempt, not sending any packet in
    /// response.
    pub fn ignore(mut self) {
        let inner = self.0.take().unwrap();
        inner.endpoint.ignore(inner.incoming);
    }

    /// The local IP address which was used when the peer established
    /// the connection.
    pub fn local_ip(&self) -> Option<IpAddr> {
        self.0.as_ref().unwrap().incoming.local_ip()
    }

    /// The peer's UDP address.
    pub fn remote_address(&self) -> SocketAddr {
        self.0.as_ref().unwrap().incoming.remote_address()
    }

    /// Whether the socket address that is initiating this connection has
    /// been validated.
    ///
    /// This means that the sender of the initial packet has proved that
    /// they can receive traffic sent to `self.remote_address()`.
    pub fn remote_address_validated(&self) -> bool {
        self.0.as_ref().unwrap().incoming.remote_address_validated()
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        // Implicit reject, similar to Connection's implicit close
        if let Some(inner) = self.0.take() {
            inner.endpoint.refuse(inner.incoming);
        }
    }
}

/// Error for attempting to retry an [`Incoming`] which already bears an
/// address validation token from a previous retry.
#[derive(Debug, Error)]
#[error("retry() with validated Incoming")]
pub struct RetryError(Incoming);

impl RetryError {
    /// Get the [`Incoming`]
    pub fn into_incoming(self) -> Incoming {
        self.0
    }
}

/// Basic adapter to let [`Incoming`] be `await`-ed like a [`Connecting`].
#[derive(Debug)]
pub struct IncomingFuture(Result<Connecting, ConnectionError>);

impl Future for IncomingFuture {
    type Output = Result<Connection, ConnectionError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match &mut self.0 {
            Ok(connecting) => connecting.poll_unpin(cx),
            Err(e) => Poll::Ready(Err(e.clone())),
        }
    }
}

impl IntoFuture for Incoming {
    type IntoFuture = IncomingFuture;
    type Output = Result<Connection, ConnectionError>;

    fn into_future(self) -> Self::IntoFuture {
        IncomingFuture(self.accept())
    }
}
