//! Provides [`MaybeTlsStream`].

#[cfg(feature = "rustls")]
pub use compio_tls::MaybeTlsStream;
