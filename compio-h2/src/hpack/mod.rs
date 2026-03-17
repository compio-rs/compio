/// HPACK header block decoder (RFC 7541).
pub mod decoder;
/// HPACK header block encoder (RFC 7541).
pub mod encoder;
/// Huffman coding tables and routines for HPACK.
pub mod huffman;
/// Static and dynamic header table implementation.
pub mod table;

/// Re-export the HPACK decoder and decoded header type.
pub use self::decoder::{DecodedHeader, Decoder};
/// Re-export the HPACK encoder.
pub use self::encoder::Encoder;
