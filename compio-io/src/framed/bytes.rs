//! Bytes-framed connections.
//! 
//! This module provides a BytesFramed type alias for creating bytes-framed connections.
//! 
//! # Examples
//! 
//! ```rust, compile_fail
//! use std::io::Cursor;
//! use compio_io::framed::bytes::BytesFramed;
//! use compio_fs::File;
//! use compio_buf::bytes::Bytes;
//! 
//! async fn example() {
//!     let file = File::open("test.txt").await.unwrap();
//!     let mut framed = BytesFramed::from_reader(Cursor::new(file));
//!     let data = framed.next().await.unwrap();
//!     assert_eq!(data, Bytes::from("Hello, world!"));
//! }
//! ```
use compio_buf::bytes::Bytes;
use futures_util::Stream;

use crate::{
    framed::{Framed, SymmetricFramed, codec::bytes::BytesCodec, frame::NoopFramer},
    read::AsyncRead,
};

/// A type alias for a framed connection using the bytes codec.
pub type BytesFramed<R> = SymmetricFramed<R, (), BytesCodec, NoopFramer, Bytes>;

impl<R> BytesFramed<R>
where
    R: AsyncRead + 'static + Unpin,
{
    /// Create a bytes-framed stream from a reader.
    pub fn from_reader(reader: R) -> Self {
        let framer = NoopFramer::new();
        Framed::symmetric(BytesCodec::new(), framer).with_reader(reader)
    }

    /// Convert this readable stream into a bytes-framed stream.    
    pub fn bytes(self) -> Self
    where
        Self: Stream<Item = Result<Bytes, std::io::Error>>,
    {
        self
    }
}
