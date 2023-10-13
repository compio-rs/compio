//! # Compio IO
//!
//! This crate provides traits and utilities for completion-based IO.
//!
//! ## Fundamentation
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
//! let buf = Vec::with_capacity(20);
//! let BufResult(res, buf) = reader.read(buf).await;
//!
//! assert!(buf.as_slice() == reader);
//! assert!(res.unwrap() == 13);
//! # }
#![feature(async_fn_in_trait)] // Remove this when AFIT is stable

mod buffer;
mod read;
mod util;
mod write;

pub(crate) use std::io::Result as IoResult;

pub use compio_buf as buf;
pub use read::*;
pub use write::*;
