//!
#![doc = include_str!("../../README.md")]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![cfg_attr(feature = "read_buf", feature(read_buf))]
#![warn(missing_docs)]

pub mod fs;
pub mod net;

pub use buf::BufResult;
pub use compio_buf as buf;

#[cfg(target_os = "windows")]
pub mod named_pipe;

#[cfg(feature = "runtime")]
mod attacher;
#[cfg(feature = "event")]
pub mod event;
#[cfg(feature = "runtime")]
pub(crate) use attacher::Attacher;
#[cfg(feature = "signal")]
pub mod signal;
#[cfg(feature = "macros")]
pub use compio_macros::*;
