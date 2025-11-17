//! Provides [`MaybeTlsStream`].

#![deprecated = "Use `compio-tls` crate instead."]

#[cfg(feature = "rustls")]
pub use compio_tls::MaybeTlsStream;
