//! Native HTTP/2 implementation built on compio's completion-based I/O.
//!
//! Provides a full HTTP/2 wire codec, HPACK header compression, flow control,
//! and client/server connection management without depending on tokio or the
//! `h2` crate.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────┐
//! │  client.rs / server.rs / builder.rs   (API)    │
//! ├────────────────────────────────────────────────┤
//! │  proto/connection.rs    (connection event loop) │
//! │  proto/streams.rs       (stream state machine) │
//! │  proto/flow_control.rs  (window tracking)       │
//! │  proto/settings.rs      (SETTINGS state)        │
//! │  proto/ping_pong.rs     (keepalive)             │
//! ├────────────────────────────────────────────────┤
//! │  codec/reader.rs        (frame I/O ← compio)   │
//! │  codec/writer.rs        (frame I/O → compio)   │
//! ├────────────────────────────────────────────────┤
//! │  frame/*   (pure frame encode/decode)           │  ← runtime-agnostic
//! │  hpack/*   (pure HPACK encode/decode)           │  ← runtime-agnostic
//! └────────────────────────────────────────────────┘
//! ```
//!
//! Roughly 45% of the crate (frame/ + hpack/) is pure protocol logic with
//! **zero** async or runtime dependencies. The I/O boundary is narrow: only
//! `codec/reader.rs` and `codec/writer.rs` use
//! [`compio_io::AsyncRead`]/[`compio_io::AsyncWrite`].

#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(unused_features)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

/// Builder pattern for configuring client and server connections.
pub mod builder;
/// HTTP/2 client API.
pub mod client;
/// Error types for HTTP/2 operations.
pub mod error;
/// HTTP/2 frame types (DATA, HEADERS, SETTINGS, etc.).
pub mod frame;
/// HPACK header compression (RFC 7541).
pub mod hpack;
/// HTTP/2 server API.
pub mod server;
/// Shared stream handles ([`SendStream`] and [`RecvStream`]).
pub mod share;

/// Frame codec for reading and writing HTTP/2 frames.
pub(crate) mod codec;
/// Protocol-level connection management.
pub(crate) mod proto;
/// Shared connection state (direct state machine access pattern).
pub(crate) mod state;

/// Re-export client and server connection builders.
pub use builder::{ClientBuilder, ServerBuilder};
/// Re-export error types.
pub use error::{FrameError, H2Error, HpackError, Reason};
/// Re-export frame types.
pub use frame::{Frame, FrameHeader, StreamId};
/// Re-export HPACK codec types.
pub use hpack::{DecodedHeader, Decoder as HpackDecoder, Encoder as HpackEncoder};
/// Re-export the PING/PONG keepalive mechanism.
pub use proto::ping_pong::PingPong;
/// Re-export connection settings.
pub use proto::settings::ConnSettings;
/// Re-export stream send/receive handles and flow control.
pub use share::{RecvFlowControl, RecvStream, SendStream};

/// TLS support for HTTP/2 connections.
#[cfg(feature = "tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
pub mod tls;
