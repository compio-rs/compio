//! # Compio
//! A thread-per-core Rust runtime with IOCP/io_uring/polling.
//! The name comes from "completion-based IO".
//! This crate is inspired by [monoio](https://github.com/bytedance/monoio/).
//!
//! ## Quick start
//! ```rust
//! # compio::runtime::Runtime::new().unwrap().block_on(async {
//! use compio::{fs::File, io::AsyncReadAtExt};
//!
//! let file = File::open("Cargo.toml").await.unwrap();
//! let (read, buffer) = file
//!     .read_to_end_at(Vec::with_capacity(1024), 0)
//!     .await
//!     .unwrap();
//! assert_eq!(read, buffer.len());
//! let buffer = String::from_utf8(buffer).unwrap();
//! println!("{}", buffer);
//! # })
//! ```

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

#[doc(no_inline)]
pub use buf::BufResult;
#[cfg(feature = "arrayvec")]
pub use buf::arrayvec;
#[cfg(feature = "bumpalo")]
pub use buf::bumpalo;
#[cfg(feature = "bytes")]
pub use buf::bytes;
#[cfg(feature = "dispatcher")]
#[doc(inline)]
pub use compio_dispatcher as dispatcher;
#[cfg(feature = "io")]
#[doc(inline)]
pub use compio_io as io;
#[cfg(feature = "macros")]
pub use compio_macros::*;
#[cfg(feature = "process")]
#[doc(inline)]
pub use compio_process as process;
#[cfg(feature = "quic")]
#[doc(inline)]
pub use compio_quic as quic;
#[cfg(feature = "signal")]
#[doc(inline)]
pub use compio_signal as signal;
#[cfg(feature = "tls")]
#[doc(inline)]
pub use compio_tls as tls;
#[cfg(feature = "utils")]
#[doc(inline)]
pub use compio_utils as utils;
#[cfg(feature = "event")]
#[doc(no_inline)]
pub use runtime::event;
#[cfg(feature = "time")]
#[doc(no_inline)]
pub use runtime::time;
#[cfg(feature = "native-tls")]
pub use tls::native_tls;
#[cfg(feature = "rustls")]
pub use tls::rustls;
#[doc(inline)]
pub use {compio_buf as buf, compio_driver as driver};
#[cfg(feature = "runtime")]
#[doc(inline)]
pub use {compio_fs as fs, compio_net as net, compio_runtime as runtime};
