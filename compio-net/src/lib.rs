//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

mod resolve;
mod socket;
mod tcp;
mod udp;
mod unix;

pub use resolve::ToSocketAddrsAsync;
pub(crate) use resolve::{each_addr, first_addr_buf};
pub(crate) use socket::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;
