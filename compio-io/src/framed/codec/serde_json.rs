//! [`Encoder`]/[`Decoder`] implementation with serde_json
//!
//! This module provides a codec implementation for JSON serialization and
//! deserialization using serde_json.
//!
//! # Examples
//!
//! ```
//! use compio_io::framed::codec::{Decoder, Encoder, serde_json::SerdeJsonCodec};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct Person {
//!     name: String,
//!     age: u32,
//! }
//!
//! let mut codec = SerdeJsonCodec::new();
//! let person = Person {
//!     name: "Alice".to_string(),
//!     age: 30,
//! };
//!
//! // Encoding
//! let mut buffer = Vec::new();
//! codec.encode(person, &mut buffer).unwrap();
//!
//! // Decoding
//! let decoded: Person = codec.decode(&buffer).unwrap();
//! assert_eq!(decoded.name, "Alice");
//! assert_eq!(decoded.age, 30);
//! ```
//!
//! [`Encoder`]: crate::framed::codec::Encoder
//! [`Decoder`]: crate::framed::codec::Decoder

use std::io;

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::framed::codec::{Decoder, Encoder};

/// A codec for JSON serialization and deserialization using serde_json.
///
/// This codec can be configured to output pretty-printed JSON by setting the
/// `pretty` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SerdeJsonCodec {
    pretty: bool,
}

impl SerdeJsonCodec {
    /// Creates a new `SerdeJsonCodec` with default settings (not
    /// pretty-printed).
    pub fn new() -> Self {
        Self { pretty: false }
    }

    /// Creates a new `SerdeJsonCodec` with pretty-printing enabled.
    pub fn pretty() -> Self {
        Self { pretty: true }
    }

    /// Sets whether the JSON output should be pretty-printed.
    pub fn set_pretty(&mut self, pretty: bool) -> &mut Self {
        self.pretty = pretty;
        self
    }

    /// Returns whether pretty-printing is enabled.
    pub fn is_pretty(&self) -> bool {
        self.pretty
    }
}

impl Default for SerdeJsonCodec {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during JSON encoding or decoding.
#[derive(Debug, Error)]
pub enum SerdeJsonCodecError {
    /// Error from serde_json during serialization or deserialization.
    #[error("serde-json error: {0}")]
    SerdeJsonError(serde_json::Error),

    /// I/O error during encoding or decoding.
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
}

impl<T: Serialize> Encoder<T> for SerdeJsonCodec {
    type Error = SerdeJsonCodecError;

    fn encode(&mut self, item: T, buf: &mut Vec<u8>) -> Result<(), Self::Error> {
        if self.pretty {
            serde_json::to_writer_pretty(buf, &item)
        } else {
            serde_json::to_writer(buf, &item)
        }
        .map_err(SerdeJsonCodecError::SerdeJsonError)
    }
}

impl<T: DeserializeOwned> Decoder<T> for SerdeJsonCodec {
    type Error = SerdeJsonCodecError;

    fn decode(&mut self, buf: &[u8]) -> Result<T, Self::Error> {
        serde_json::from_slice(buf).map_err(SerdeJsonCodecError::SerdeJsonError)
    }
}
