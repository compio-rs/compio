//! Utilities for working with file descriptors.

mod poll_fd;
pub use poll_fd::*;

#[cfg(feature = "async-fd")]
mod async_fd;
#[cfg(feature = "async-fd")]
pub use async_fd::*;
