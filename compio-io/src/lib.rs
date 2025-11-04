//! This crate provides traits and utilities for completion-based IO.
//!
//! # Contents
//! ### Fundamental
//!
//! - [`AsyncRead`]: Async read into a buffer implements [`IoBufMut`]
//! - [`AsyncReadAt`]: Async read into a buffer implements [`IoBufMut`] with
//!   offset
//! - [`AsyncWrite`]: Async write from a buffer implements [`IoBuf`]
//! - [`AsyncWriteAt`]: Async write from a buffer implements [`IoBuf`] with
//!   offset
//!
//! ### Buffered IO
//!
//! - [`AsyncBufRead`]: Trait of async read with buffered content
//! - [`BufReader`]: An async reader with internal buffer
//! - [`BufWriter`]: An async writer with internal buffer
//!
//! ### Extension
//!
//! - [`AsyncReadExt`]: Extension trait for [`AsyncRead`]
//! - [`AsyncReadAtExt`]: Extension trait for [`AsyncReadAt`]
//! - [`AsyncWriteExt`]: Extension trait for [`AsyncWrite`]
//! - [`AsyncWriteAtExt`]: Extension trait for [`AsyncWriteAt`]
//!
//! ### Adapters
//! - [`framed::Framed`]: Adapts [`AsyncRead`] to [`Stream`] and [`AsyncWrite`]
//!   to [`Sink`], with framed de/encoding.
//! - [`compat::SyncStream`]: Adapts async IO to std blocking io (requires
//!   `compat` feature)
//! - [`compat::AsyncStream`]: Adapts async IO to [`futures_util::io`] traits
//!   (requires `compat` feature)
//!
//! [`IoBufMut`]: compio_buf::IoBufMut
//! [`IoBuf`]: compio_buf::IoBuf
//! [`Sink`]: futures_util::Sink
//! [`Stream`]: futures_util::Stream
//!
//! # Examples
//!
//! ### Read
//!
//! ```
//! use compio_buf::BufResult;
//! use compio_io::AsyncRead;
//! # compio_runtime::Runtime::new().unwrap().block_on(async {
//!
//! let mut reader = "Hello, world!".as_bytes();
//! let (res, buf) = reader.read(Vec::with_capacity(20)).await.unwrap();
//!
//! assert_eq!(buf.as_slice(), b"Hello, world!");
//! assert_eq!(res, 13);
//! # })
//! ```
//!
//! ### Write
//!
//! Writing to a fixed-size buffer wrapped by [`Cursor`](std::io::Cursor). The
//! implementation will write the content start at the current
//! [`position`](std::io::Cursor::position):
//!
//! ```
//! use std::io::Cursor;
//!
//! use compio_buf::BufResult;
//! use compio_io::AsyncWrite;
//! # compio_runtime::Runtime::new().unwrap().block_on(async {
//!
//! let mut writer = Cursor::new([0; 6]);
//! writer.set_position(2);
//! let (n, buf) = writer.write(vec![1, 1, 1, 1, 1, 1]).await.unwrap();
//!
//! assert_eq!(n, 4);
//! assert_eq!(writer.into_inner(), [0, 0, 1, 1, 1, 1]);
//! # })
//! ```
//!
//! Writing to `Vec<u8>`, which is extendable. Notice that the implementation
//! will append the content to the end:
//!
//! ```
//! use compio_buf::BufResult;
//! use compio_io::AsyncWrite;
//! # compio_runtime::Runtime::new().unwrap().block_on(async {
//!
//! let mut writer = vec![1, 2, 3];
//! let (_, buf) = writer.write(vec![3, 2, 1]).await.unwrap();
//!
//! assert_eq!(writer, [1, 2, 3, 3, 2, 1]);
//! # })
//! ```
//!
//! This crate doesn't depend on a specific runtime. It can work with `tokio`
//! well:
//! ```
//! use compio_buf::BufResult;
//! use compio_io::AsyncWrite;
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() {
//!     let mut writer = vec![1, 2, 3];
//!     let (_, buf) = writer.write(vec![3, 2, 1]).await.unwrap();
//!
//!     assert_eq!(writer, [1, 2, 3, 3, 2, 1]);
//! }
//! ```

#![warn(missing_docs)]
// This is OK as we're thread-per-core and don't need `Send` or other auto trait on anonymous future
#![allow(async_fn_in_trait)]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::{future::Future, pin::Pin};
type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

mod buffer;
pub mod framed;

#[cfg(feature = "compat")]
pub mod compat;
mod read;
pub mod util;
mod write;

pub(crate) type IoResult<T> = std::io::Result<T>;

pub use read::*;
#[doc(inline)]
pub use util::{
    copy, null, repeat,
    split::{split, split_unsync},
};
pub use write::*;
