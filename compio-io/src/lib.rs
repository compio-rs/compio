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
//!
//! [`IoBufMut`]: compio_buf::IoBufMut
//! [`IoBuf`]: compio_buf::IoBuf
//!
//! # Examples
//!
//! ### Read
//!
//! ```
//! use compio_buf::BufResult;
//! use compio_io::AsyncRead;
//! # #[tokio::main(flavor = "current_thread")] async fn main() {
//!
//! let mut reader = "Hello, world!".as_bytes();
//! let (res, buf) = reader.read(Vec::with_capacity(20)).await.unwrap();
//!
//! assert!(buf.as_slice() == reader);
//! assert!(res == 13);
//! # }
//! ```
//!
//! ### Write
//!
//! Writing to a fixed-size buffer wrapped by [`Cursor`]. The
//! implementation will write the content start at the current
//! [`position`](std::io::Cursor::position):
//!
//! ```
//! use std::io::Cursor;
//!
//! use compio_buf::BufResult;
//! use compio_io::AsyncWrite;
//! # #[tokio::main(flavor = "current_thread")] async fn main() {
//!
//! let mut writer = Cursor::new([0; 6]);
//! writer.set_position(2);
//! let (n, buf) = writer.write(vec![1, 1, 1, 1, 1, 1]).await.unwrap();
//!
//! assert_eq!(n, 4);
//! assert_eq!(writer.into_inner(), [0, 0, 1, 1, 1, 1]);
//! # }
//! ```
//!
//! Writing to `Vec<u8>`, which is extendable. Notice that the implentation will
//! append the content to the end:
//!
//! ```
//! use compio_buf::BufResult;
//! use compio_io::AsyncWrite;
//! # #[tokio::main(flavor = "current_thread")] async fn main() {
//!
//! let mut writer = vec![1, 2, 3];
//! let (_, buf) = writer.write(vec![3, 2, 1]).await.unwrap();
//!
//! assert_eq!(writer, [1, 2, 3, 3, 2, 1]);
//! # }
//! ```
//!
//! [`Cursor`]: std::io::Cursor

#![warn(missing_docs)]
// This is OK as we're thread-per-core and don't need `Send` or other auto trait on anonymous future
#![allow(async_fn_in_trait)]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

mod buffer;
mod read;
pub mod util;
mod write;

pub(crate) type IoResult<T> = std::io::Result<T>;

pub use buffer::Buffer;
pub use read::*;
pub use util::{copy, null, repeat};
pub use write::*;
