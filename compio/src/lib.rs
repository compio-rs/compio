//! # Compio
//! A thread-per-core Rust runtime with IOCP/io_uring/polling.
//! The name comes from "completion-based IO".
//! This crate is inspired by [monoio](https://github.com/bytedance/monoio/).
//!
//! ## Quick start
//! ```rust
//! # compio::runtime::block_on(async {
//! use compio::fs::File;
//!
//! let file = File::open("Cargo.toml").unwrap();
//! let (read, buffer) = file
//!     .read_to_end_at(Vec::with_capacity(1024), 0)
//!     .await
//!     .unwrap();
//! assert_eq!(read, buffer.len());
//! let buffer = String::from_utf8(buffer).unwrap();
//! println!("{}", buffer);
//! # })
//! ```

#![warn(missing_docs)]

#[cfg(feature = "macros")]
#[doc(no_inline)]
pub use compio_macros::*;
#[cfg(feature = "runtime")]
#[doc(no_inline)]
pub use compio_runtime as runtime;
#[cfg(feature = "event")]
#[doc(no_inline)]
pub use compio_runtime::event;
#[cfg(feature = "time")]
#[doc(no_inline)]
pub use compio_runtime::time;
#[cfg(feature = "signal")]
#[doc(no_inline)]
pub use compio_signal as signal;
#[doc(no_inline)]
pub use {
    compio_buf::{self as buf, BufResult},
    compio_driver as driver,
    compio_fs::{self as fs},
    compio_net as net,
};
