//! # Compio IO
//!
//! This crate provides traits and utilities for completion-based IO.
//!
//! ## Fundamental
//!
//! - [`AsyncRead`]: Async read into a buffer implements [`IoBufMut`]
//! - [`AsyncWrite`]: Async write from a buffer implements [`IoBuf`]
//!
//! ## Buffered IO
//!
//! - [`AsyncBufRead`]: Trait of async read with buffered content
//! - [`BufReader`]: An async reader with internal buffer
//! - [`BufWriter`]: An async writer with internal buffer
//!
//! [`IoBufMut`]: compio_buf::IoBufMut
//! [`IoBuf`]: compio_buf::IoBuf
//!
//! ## Examples
//!
//! ### Read
//!
//! ```rust
//! use compio_io::{
//!     buf::BufResult,
//!     AsyncBufRead, AsyncRead, BufReader
//! };
//! # #[compio_macros::main] async fn main() {
//!
//! let mut reader = "Hello, world!".as_bytes();
//! let (res, buf) = reader.read(Vec::with_capacity(20)).await.unwrap();
//!
//! assert!(buf.as_slice() == reader);
//! assert!(res == 13);
//! # }
// Remove this when AFIT is stable
#![feature(async_fn_in_trait)]
// This is OK as we're thread-per-core which doesn't need `Send` or other auto trait on anonymous
// future
#![allow(async_fn_in_trait)]

mod buffer;
mod read;
pub mod util;
mod write;

pub(crate) type IoResult<T> = std::io::Result<T>;

#[doc(inline)]
pub use compio_buf as buf;
pub use read::*;
#[doc(inline)]
pub use util::{copy, null, repeat};
pub use write::*;
