//! Traits and implementations for encoding/decoding structured types to/from
//! bytes.

use std::io;

#[cfg(feature = "codec-serde-json")]
pub mod serde_json;

/// Trait for types that encode values into bytes.
pub trait Encoder<Item> {
    /// The error type that can be returned during encoding operations.
    type Error: From<io::Error>;

    /// Encodes an item into bytes.
    ///
    /// The `buf` is *guaranteed* to have 0 initialized bytes (`len` == 0). At
    /// the end, all initialized bytes will be treated as valid content to
    /// be transimitted.
    fn encode(&mut self, item: Item, buf: &mut Vec<u8>) -> Result<(), Self::Error>;
}

/// Trait for decoding byte sequences back into structured items.
pub trait Decoder<Item> {
    /// Errors happened during the decoding process
    type Error: From<io::Error>;

    /// Decodes a byte sequence into an item.
    fn decode(&mut self, buf: &[u8]) -> Result<Item, Self::Error>;
}
