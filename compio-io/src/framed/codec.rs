//! This module defines traits for encoding/decoding structured types to/from
//! bytes.

use std::io;

/// Trait for types that encode values into bytes.
pub trait Encoder<Item> {
    /// The error type that can be returned during encoding operations.
    type Error: From<io::Error>;

    /// Encodes an item into bytes.
    ///
    /// Returns the number of bytes written to the buffer.
    fn encode(&self, item: Item, buf: &mut Vec<u8>) -> Result<(), Self::Error>;
}

/// Trait for decoding byte sequences back into structured items.
pub trait Decoder {
    /// The type of item this decoder can extract.
    type Item;

    /// Errors happened during the decoding process
    type Error: From<io::Error>;

    /// Decodes a byte sequence into an item.
    fn decode(&self, buf: &[u8]) -> Result<Self::Item, Self::Error>;
}
