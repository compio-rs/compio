//! TLS streams.

#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "native-tls")]
pub use native_tls;
#[cfg(feature = "rustls")]
pub use rustls;

mod adapter;
mod maybe;
mod stream;

pub use adapter::*;
pub use maybe::*;
pub use stream::*;
