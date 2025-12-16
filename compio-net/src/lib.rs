//! Network utilities.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod cmsg;
mod opts;
mod poll_fd;
mod resolve;
mod socket;
pub(crate) mod split;
mod tcp;
mod udp;
mod unix;

pub use cmsg::*;
pub use opts::SocketOpts;
pub use poll_fd::*;
pub use resolve::ToSocketAddrsAsync;
pub(crate) use resolve::{each_addr, first_addr_buf};
pub(crate) use socket::*;
pub use split::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;
