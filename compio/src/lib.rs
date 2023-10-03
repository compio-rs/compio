//!
#![doc = include_str!("../../README.md")]
#![warn(missing_docs)]

#[cfg(target_os = "windows")]
#[doc(no_inline)]
pub use compio_fs::named_pipe;
#[cfg(feature = "macros")]
#[doc(no_inline)]
pub use compio_macros::*;
#[cfg(feature = "runtime")]
#[doc(no_inline)]
pub use compio_runtime as task;
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
