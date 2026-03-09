//! [`Encoder`]/[`Decoder`] implementation with Bytes
//!
//! This module provides a codec implementation for bytes serialization and
//! deserialization (noop per se).
//!
//! # Examples
//!
//! ```
//! use compio_buf::IoBuf;
//! use compio_io::framed::codec::{Decoder, Encoder, bytes::BytesCodec};
//! use compio_buf::bytes::Bytes;
//!
//! let mut codec = BytesCodec::new();
//! let data = Bytes::from("Hello, world!");
//!
//! // Encoding
//! let mut buffer = Vec::new();
//! codec.encode(data.clone(), &mut buffer).unwrap();
//!
//! // Decoding
//! let decoded = codec.decode(&buffer.as_slice()).unwrap();
//! assert_eq!(decoded, data);
//! ```
use std::io::{self, Write};

use compio_buf::{IoBuf, IoBufMut, Slice, bytes::Bytes};

use crate::framed::codec::{Decoder, Encoder};

/// A codec for bytes serialization and deserialization.
///
/// This codec can be used to write into and read from [`Bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BytesCodec;

impl BytesCodec {
    /// Creates a new `BytesCodec`.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for BytesCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: IoBufMut> Encoder<Bytes, B> for BytesCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Bytes, buf: &mut B) -> Result<(), Self::Error> {
        let mut writer = buf.as_writer();
        writer.write_all(&item)?;
        Ok(())
    }
}

impl<B: IoBuf> Decoder<Bytes, B> for BytesCodec {
    type Error = io::Error;

    fn decode(&mut self, buf: &Slice<B>) -> Result<Bytes, Self::Error> {
        let inner = buf.as_ref().to_vec();
        Ok(Bytes::from(inner))
    }
}
