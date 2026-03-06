//! The compio runtime.
//!
//! ```
//! let ans = compio_runtime::Runtime::new().unwrap().block_on(async {
//!     println!("Hello world!");
//!     42
//! });
//! assert_eq!(ans, 42);
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "current_thread_id", feature(current_thread_id))]
#![cfg_attr(feature = "future-combinator", feature(context_ext, local_waker))]
#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

mod affinity;
mod attacher;
mod cancel;
pub mod fd;
mod runtime;

#[cfg(feature = "future-combinator")]
pub mod future;
#[cfg(feature = "time")]
pub mod time;

pub use async_task::Task;
pub use attacher::*;
pub use cancel::CancelToken;
use compio_buf::BufResult;
#[allow(hidden_glob_reexports, unused)]
use runtime::RuntimeInner; // used to shadow glob export so that RuntimeInner is not exported
pub use runtime::*;
