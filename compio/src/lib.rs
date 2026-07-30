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
//!
//! ## Linux `io_uring` tuning
//!
//! [`runtime::Runtime`] is thread-local and cannot be sent between threads. Create a
//! runtime inside each worker thread when using Compio's thread-per-core model. For
//! dispatching work across runtime threads, use the
//! [`dispatcher`](https://docs.rs/compio-dispatcher) crate.
//!
//! On Linux, applications that enable Compio's `io-uring` driver can opt in to kernel
//! features that reduce task-work overhead. These settings are workload-dependent, so
//! benchmark them in the deployment environment rather than treating them as universal
//! defaults:
//!
//! ```rust
//! use compio::driver::ProactorBuilder;
//! use compio::runtime::RuntimeBuilder;
//!
//! let mut proactor = ProactorBuilder::new();
//! proactor
//!     // Available on Linux 6.0 and later.
//!     .single_issuer(true)
//!     // Available on Linux 5.19 and later.
//!     .coop_taskrun(true)
//!     .taskrun_flag(true);
//!
//! let runtime = RuntimeBuilder::new()
//!     .with_proactor(proactor)
//!     .build()
//!     .unwrap();
//! ```
//!
//! These options are effective only with the `io-uring` feature. `taskrun_flag` should
//! be used with `coop_taskrun`; `defer_taskrun` is a separate Linux 6.1+ option that
//! requires `single_issuer`.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

#[doc(no_inline)]
pub use buf::BufResult;
#[cfg(feature = "arrayvec")]
pub use buf::arrayvec;
#[cfg(feature = "bumpalo")]
pub use buf::bumpalo;
#[cfg(feature = "bytes")]
pub use buf::bytes;
#[cfg(feature = "smallvec")]
pub use buf::smallvec;
#[cfg(feature = "compat")]
pub use compio_compat as compat;
#[cfg(feature = "dispatcher")]
#[doc(inline)]
pub use compio_dispatcher as dispatcher;
#[cfg(feature = "fs")]
#[doc(inline)]
pub use compio_fs as fs;
#[cfg(feature = "io")]
#[doc(inline)]
pub use compio_io as io;
#[cfg(feature = "macros")]
pub use compio_macros::*;
#[cfg(feature = "net")]
#[doc(inline)]
pub use compio_net as net;
#[cfg(feature = "process")]
#[doc(inline)]
pub use compio_process as process;
#[cfg(feature = "quic")]
#[doc(inline)]
pub use compio_quic as quic;
#[cfg(feature = "runtime")]
#[doc(inline)]
pub use compio_runtime as runtime;
#[cfg(feature = "signal")]
#[doc(inline)]
pub use compio_signal as signal;
#[cfg(feature = "tls")]
#[doc(inline)]
pub use compio_tls as tls;
#[cfg(feature = "ws")]
#[doc(inline)]
pub use compio_ws as ws;
#[cfg(feature = "time")]
#[doc(inline)]
pub use runtime::time;
#[cfg(feature = "native-tls")]
pub use tls::native_tls;
#[cfg(feature = "rustls")]
pub use tls::rustls;
#[doc(inline)]
pub use {compio_buf as buf, compio_driver as driver};
