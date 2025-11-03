//! Async TLS streams.

#![warn(missing_docs)]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(all(not(feature = "native-tls"), not(feature = "rustls")))]
compile_error!("You must choose at least one of these features: [\"native-tls\", \"rustls\"]");

#[cfg(feature = "native-tls")]
pub use native_tls;
#[cfg(feature = "rustls")]
pub use rustls;

mod adapter;
mod stream;

pub use adapter::*;
pub use stream::*;
