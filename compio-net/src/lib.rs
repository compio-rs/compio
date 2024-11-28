//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]
#![allow(unsafe_op_in_unsafe_fn)]

mod cmsg;
mod poll_fd;
mod resolve;
mod socket;
pub(crate) mod split;
mod tcp;
mod udp;
mod unix;

pub use cmsg::*;
pub use poll_fd::*;
pub use resolve::ToSocketAddrsAsync;
pub(crate) use resolve::{each_addr, first_addr_buf};
pub(crate) use socket::*;
pub use split::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;
