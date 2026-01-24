//! Traits and implementations for encoding/decoding structured types to/from
//! bytes.

use std::io;

use compio_buf::{IoBuf, IoBufMut, Slice};

#[cfg(feature = "codec-serde-json")]
pub mod serde_json;

/// Trait for types that encode values into bytes.
pub trait Encoder<Item, B: IoBufMut> {
    /// The error type that can be returned during encoding operations.
    type Error: From<io::Error>;

    /// Encodes an item into bytes.
    ///
    /// The `buf` is *guaranteed* to have 0 initialized bytes (`buf_len()` ==
    /// 0). If the function is returned successfully, all initialized bytes will
    /// be treated as valid content to be transmitted.
    fn encode(&mut self, item: Item, buf: &mut B) -> Result<(), Self::Error>;
}

/// Trait for decoding byte sequences back into structured items.
pub trait Decoder<Item, B: IoBuf> {
    /// Errors happened during the decoding process
    type Error: From<io::Error>;

    /// Decodes a byte sequence into an item.
    ///
    /// The given `buf` is a sliced view into the underlying buffer, which gives
    /// one complete frame. [`Slice`] implements [`IoBuf`].
    ///
    /// You may escape the view by calling [`Slice::as_inner`].
    fn decode(&mut self, buf: &Slice<B>) -> Result<Item, Self::Error>;
}
