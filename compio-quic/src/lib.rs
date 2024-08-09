//! QUIC implementation for compio
//!
//! Ported from [`quinn`].
//!
//! [`quinn`]: https://docs.rs/quinn

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

pub use quinn_proto::{
    congestion, crypto, AckFrequencyConfig, ApplicationClose, Chunk, ClientConfig, ClosedStream,
    ConfigError, ConnectError, ConnectionClose, ConnectionError, ConnectionStats, EndpointConfig,
    IdleTimeout, MtuDiscoveryConfig, ServerConfig, StreamId, Transmit, TransportConfig, VarInt,
};

mod builder;
mod connection;
mod endpoint;
mod incoming;
mod recv_stream;
mod send_stream;
mod socket;

pub use builder::{ClientBuilder, ServerBuilder};
pub use connection::{Connecting, Connection};
pub use endpoint::Endpoint;
pub use incoming::{Incoming, IncomingFuture};
pub use recv_stream::{ReadError, RecvStream};
pub use send_stream::{SendStream, WriteError};

pub(crate) use crate::{
    connection::{ConnectionEvent, ConnectionInner},
    endpoint::EndpointInner,
    socket::*,
};

/// Errors from [`SendStream::stopped`] and [`RecvStream::stopped`].
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum StoppedError {
    /// The connection was lost
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
    /// This was a 0-RTT stream and the server rejected it
    ///
    /// Can only occur on clients for 0-RTT streams, which can be opened using
    /// [`Connecting::into_0rtt()`].
    ///
    /// [`Connecting::into_0rtt()`]: crate::Connecting::into_0rtt()
    #[error("0-RTT rejected")]
    ZeroRttRejected,
}

impl From<StoppedError> for std::io::Error {
    fn from(x: StoppedError) -> Self {
        use StoppedError::*;
        let kind = match x {
            ZeroRttRejected => std::io::ErrorKind::ConnectionReset,
            ConnectionLost(_) => std::io::ErrorKind::NotConnected,
        };
        Self::new(kind, x)
    }
}

macro_rules! wait_event {
    ($event:expr, $break:expr) => {
        loop {
            $break;
            event_listener::listener!($event => listener);
            $break;
            listener.await;
        }
    };
}
pub(crate) use wait_event;
