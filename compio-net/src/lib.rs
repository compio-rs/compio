//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "runtime")]
mod resolve;
mod socket;
mod tcp;
mod udp;
mod unix;

#[cfg(feature = "runtime")]
pub(crate) use resolve::each_addr;
#[cfg(feature = "runtime")]
pub use resolve::ToSocketAddrsAsync;
pub(crate) use socket::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;
