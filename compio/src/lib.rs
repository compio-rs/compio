//!
#![doc = include_str!("../../README.md")]
#![warn(missing_docs)]

#[doc(no_inline)]
pub use compio_buf as buf;
#[doc(no_inline)]
pub use compio_driver as driver;
#[doc(no_inline)]
pub use compio_fs as fs;
#[cfg(target_os = "windows")]
#[doc(no_inline)]
pub use compio_fs::named_pipe;
#[cfg(feature = "macros")]
#[doc(no_inline)]
pub use compio_macros::*;
#[doc(no_inline)]
pub use compio_net as net;
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
