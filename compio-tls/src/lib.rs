//! Async TLS streams.

#![warn(missing_docs)]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[cfg(feature = "native-tls")]
pub use native_tls;
#[cfg(feature = "rustls")]
pub use rustls;

mod adapter;
mod stream;

pub use adapter::*;
pub use stream::*;
