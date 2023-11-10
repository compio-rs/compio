//! Async TLS streams.

#![warn(missing_docs)]
#![cfg_attr(feature = "read_buf", feature(read_buf))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod adapter;
mod stream;

pub use adapter::*;
pub use stream::*;
