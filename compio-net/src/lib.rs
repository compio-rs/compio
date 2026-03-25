//! Network utilities.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

mod incoming;
mod opts;
mod resolve;
mod socket;
pub(crate) mod split;
mod tcp;
mod udp;
mod unix;

/// Reference to a control message.
#[deprecated(
    since = "0.12.0",
    note = "use `compio_io::ancillary::AncillaryRef` instead"
)]
pub type CMsgRef<'a> = compio_io::ancillary::CMsgRef<'a>;

/// An iterator for control messages.
#[deprecated(
    since = "0.12.0",
    note = "use `compio_io::ancillary::AncillaryIter` instead"
)]
pub type CMsgIter<'a> = compio_io::ancillary::CMsgIter<'a>;

/// Helper to construct control message.
#[deprecated(
    since = "0.12.0",
    note = "use `compio_io::ancillary::AncillaryBuf::builder()` instead"
)]
pub type CMsgBuilder<'a> = compio_io::ancillary::CMsgBuilder<'a>;

/// Providing functionalities to wait for readiness.
#[deprecated(since = "0.12.0", note = "Use `compio::runtime::fd::PollFd` instead")]
pub type PollFd<T> = compio_runtime::fd::PollFd<T>;
pub(crate) use incoming::*;
pub use opts::SocketOpts;
pub use resolve::ToSocketAddrsAsync;
pub(crate) use resolve::{each_addr, first_addr_buf, first_addr_buf_zerocopy};
pub(crate) use socket::*;
pub use split::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;
