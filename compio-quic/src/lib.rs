//! QUIC implementation based on [`quinn-proto`].
//!
//! [`quinn-proto`]: https://docs.rs/quinn-proto

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

pub use quinn_proto::{
    AckFrequencyConfig, ApplicationClose, Chunk, ClientConfig, ClosedStream, ConfigError,
    ConnectError, ConnectionClose, ConnectionId, ConnectionIdGenerator, ConnectionStats, Dir,
    EcnCodepoint, EndpointConfig, FrameStats, FrameType, IdleTimeout, MtuDiscoveryConfig,
    NoneTokenLog, NoneTokenStore, PathStats, ServerConfig, Side, StdSystemTime, StreamId,
    TimeSource, TokenLog, TokenMemoryCache, TokenReuseError, TokenStore, Transmit, TransportConfig,
    TransportErrorCode, UdpStats, ValidationTokenConfig, VarInt, VarIntBoundsExceeded, Written,
    congestion, crypto,
};
#[cfg(feature = "qlog")]
pub use quinn_proto::{QlogConfig, QlogStream};

#[cfg(rustls)]
mod builder;
mod connection;
mod endpoint;
mod incoming;
mod recv_stream;
mod send_stream;
mod socket;

#[cfg(rustls)]
pub use builder::{ClientBuilder, ServerBuilder};
pub use connection::{Connecting, Connection, ConnectionError};
pub use endpoint::Endpoint;
pub use incoming::{Incoming, IncomingFuture};
pub use recv_stream::{ReadError, ReadExactError, RecvStream};
pub use send_stream::{SendStream, WriteError};
#[cfg(feature = "sync")]
pub(crate) use synchrony::sync;
#[cfg(not(feature = "sync"))]
pub(crate) use synchrony::unsync as sync;

pub(crate) use crate::{
    connection::{ConnectionEvent, ConnectionInner},
    endpoint::EndpointRef,
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

/// HTTP/3 support via [`h3`].
#[cfg(feature = "h3")]
pub mod h3 {
    pub use h3::*;

    pub use crate::{
        connection::h3_impl::{BidiStream, OpenStreams},
        send_stream::h3_impl::SendStream,
    };
}
