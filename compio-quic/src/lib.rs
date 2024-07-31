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
mod socket;

pub use builder::*;
pub(crate) use connection::ConnectionEvent;
pub use connection::*;
pub use endpoint::*;
pub use incoming::*;
pub(crate) use socket::*;
