//! The runtime of compio.
//!
//! ```
//! let ans = compio_runtime::Runtime::new().unwrap().block_on(async {
//!     println!("Hello world!");
//!     42
//! });
//! assert_eq!(ans, 42);
//! ```

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

mod attacher;
mod runtime;

#[cfg(feature = "event")]
pub mod event;
#[cfg(feature = "time")]
pub mod time;

pub use async_task::Task;
pub use attacher::*;
use compio_buf::BufResult;
pub use runtime::{
    spawn, spawn_blocking, submit, submit_with_flags, JoinHandle, Runtime, RuntimeBuilder,
};
