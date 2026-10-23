//! Utilities for working with file descriptors.

mod poll_fd;
pub use poll_fd::*;

#[cfg(feature = "async-fd")]
mod async_fd;
#[cfg(feature = "async-fd")]
pub use async_fd::*;

#[cfg(all(target_os = "linux", feature = "async-fd"))]
mod copy;
#[cfg(all(target_os = "linux", feature = "async-fd"))]
#[doc(hidden)]
pub use copy::copy_splice;
