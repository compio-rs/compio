use bytes::Bytes;

use super::{
    huffman::huffman_decode,
    table::{DynamicTable, STATIC_TABLE},
};
use crate::error::HpackError;

/// A decoded header with sensitivity flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedHeader {
    /// The header field name.
    pub name: Bytes,
    /// The header field value.
    pub value: Bytes,
    /// `true` if this header was marked as "never indexed" (Section 6.2.3).
    pub sensitive: bool,
}

/// HPACK decoder.
#[derive(Debug)]
pub struct Decoder {
    table: DynamicTable,
    /// Maximum table size allowed by SETTINGS.
    max_table_size: usize,
    /// Maximum header list size (sum of name+value+32 per header).
    /// None means unlimited.
    max_header_list_size: Option<usize>,
}

impl Decoder {
    /// Create a new HPACK decoder with the given maximum dynamic table size.
    pub fn new(max_table_size: usize) -> Self {
        Decoder {
            table: DynamicTable::new(max_table_size),
            max_table_size,
            max_header_list_size: None,
        }
    }

    /// Update the maximum dynamic table size allowed by SETTINGS.
    pub fn set_max_table_size(&mut self, size: usize) {
        self.max_table_size = size;
    }

    /// Set the maximum header list size.
    /// Each header costs name.len() + value.len() + 32 bytes (RFC 7541 §4.1).
    pub fn set_max_header_list_size(&mut self, size: usize) {
        self.max_header_list_size = Some(size);
    }

    /// The decoder's dynamic table.
    pub fn dynamic_table(&self) -> &DynamicTable {
        &self.table
    }

    /// Decode a header block into a list of decoded headers with sensitivity
    /// flags.
    pub fn decode(&mut self, src: &[u8]) -> Result<Vec<DecodedHeader>, HpackError> {
        let mut headers = Vec::new();
        let mut pos = 0;
        let mut seen_non_size_update = false;
        let mut header_list_size: usize = 0;

        while pos < src.len() {
            let byte = src[pos];

            if byte & 0x80 != 0 {
                // Indexed Header Field (Section 6.1)
                seen_non_size_update = true;
                let (index, consumed) = decode_integer(&src[pos..], 7)?;
                pos += consumed;

                let (name, value) = self.get_indexed(index as usize)?;
                header_list_size += name.len() + value.len() + 32;
                if let Some(max) = self.max_header_list_size
                    && header_list_size > max
                {
                    return Err(HpackError::HeaderListTooLarge(format!(
                        "header list size {} exceeds limit {}",
                        header_list_size, max
                    )));
                }
                headers.push(DecodedHeader {
                    name,
                    value,
                    sensitive: false,
                });
            } else if byte & 0x40 != 0 {
                // Literal Header Field with Incremental Indexing (Section 6.2.1)
                seen_non_size_update = true;
                let (index, consumed) = decode_integer(&src[pos..], 6)?;
                pos += consumed;

                let (name, value) = if index == 0 {
                    // New name
                    let (name, consumed) = decode_string(&src[pos..])?;
                    pos += consumed;
                    let (value, consumed) = decode_string(&src[pos..])?;
                    pos += consumed;
                    (name, value)
                } else {
                    // Indexed name
                    let (name, _) = self.get_indexed(index as usize)?;
                    let (value, consumed) = decode_string(&src[pos..])?;
                    pos += consumed;
                    (name, value)
                };

                header_list_size += name.len() + value.len() + 32;
                if let Some(max) = self.max_header_list_size
                    && header_list_size > max
                {
                    return Err(HpackError::HeaderListTooLarge(format!(
                        "header list size {} exceeds limit {}",
                        header_list_size, max
                    )));
                }

                self.table.insert(name.clone(), value.clone());
                headers.push(DecodedHeader {
                    name,
                    value,
                    sensitive: false,
                });
            } else if byte & 0x20 != 0 {
                // Dynamic Table Size Update (Section 6.3)
                if seen_non_size_update {
                    return Err(HpackError::TableSizeOverflow(
                        "dynamic table size update must appear at start of header block".into(),
                    ));
                }
                let (new_size, consumed) = decode_integer(&src[pos..], 5)?;
                pos += consumed;
                let new_size = new_size as usize;
                if new_size > self.max_table_size {
                    return Err(HpackError::TableSizeOverflow(format!(
                        "dynamic table size {} exceeds SETTINGS max {}",
                        new_size, self.max_table_size
                    )));
                }
                self.table.set_max_size(new_size);
            } else {
                // Literal Header Field without Indexing (Section 6.2.2): 0000xxxx
                // or Literal Header Field Never Indexed (Section 6.2.3): 0001xxxx
                seen_non_size_update = true;
                let sensitive = byte & 0x10 != 0;
                let (index, consumed) = decode_integer(&src[pos..], 4)?;
                pos += consumed;

                let (name, value) = if index == 0 {
                    let (name, consumed) = decode_string(&src[pos..])?;
                    pos += consumed;
                    let (value, consumed) = decode_string(&src[pos..])?;
                    pos += consumed;
                    (name, value)
                } else {
                    let (name, _) = self.get_indexed(index as usize)?;
                    let (value, consumed) = decode_string(&src[pos..])?;
                    pos += consumed;
                    (name, value)
                };

                header_list_size += name.len() + value.len() + 32;
                if let Some(max) = self.max_header_list_size
                    && header_list_size > max
                {
                    return Err(HpackError::HeaderListTooLarge(format!(
                        "header list size {} exceeds limit {}",
                        header_list_size, max
                    )));
                }

                // Do NOT add to dynamic table
                headers.push(DecodedHeader {
                    name,
                    value,
                    sensitive,
                });
            }
        }

        Ok(headers)
    }

    /// Get a header from the combined static + dynamic table.
    fn get_indexed(&self, index: usize) -> Result<(Bytes, Bytes), HpackError> {
        if index == 0 {
            return Err(HpackError::InvalidTableIndex("invalid index 0".into()));
        }

        if index <= STATIC_TABLE.len() {
            let (name, value) = STATIC_TABLE[index - 1];
            Ok((
                Bytes::from_static(name.as_bytes()),
                Bytes::from_static(value.as_bytes()),
            ))
        } else {
            let dyn_index = index - STATIC_TABLE.len() - 1;
            match self.table.get(dyn_index) {
                Some((name, value)) => Ok((name.clone(), value.clone())),
                None => Err(HpackError::InvalidTableIndex(format!(
                    "index {} out of range",
                    index
                ))),
            }
        }
    }
}

/// Decode an HPACK integer with the given prefix size.
/// Returns (value, bytes_consumed).
pub fn decode_integer(src: &[u8], prefix_size: u8) -> Result<(u64, usize), HpackError> {
    if src.is_empty() {
        return Err(HpackError::InvalidInteger(
            "empty input for integer decode".into(),
        ));
    }

    let max_prefix = (1u64 << prefix_size) - 1;
    let value = (src[0] as u64) & max_prefix;

    if value < max_prefix {
        return Ok((value, 1));
    }

    const MAX_CONTINUATION_BYTES: usize = 5;

    let mut value = max_prefix;
    let mut m: u32 = 0;
    let mut i = 1;
    let mut continuation_count: usize = 0;

    loop {
        if i >= src.len() {
            return Err(HpackError::InvalidInteger(
                "incomplete integer encoding".into(),
            ));
        }
        let byte = src[i] as u64;
        value += (byte & 0x7f) << m;
        m += 7;
        i += 1;
        continuation_count += 1;

        if byte & 0x80 == 0 {
            break;
        }

        if continuation_count >= MAX_CONTINUATION_BYTES {
            return Err(HpackError::InvalidInteger(
                "too many continuation bytes".into(),
            ));
        }

        if m > 63 {
            return Err(HpackError::InvalidInteger("integer overflow".into()));
        }
    }

    Ok((value, i))
}

/// Decode an HPACK string.
/// Returns (decoded_bytes, bytes_consumed).
fn decode_string(src: &[u8]) -> Result<(Bytes, usize), HpackError> {
    if src.is_empty() {
        return Err(HpackError::InvalidStringLiteral(
            "empty input for string decode".into(),
        ));
    }

    let huffman = src[0] & 0x80 != 0;
    let (length, consumed) = decode_integer(src, 7)?;
    let length = length as usize;

    if consumed + length > src.len() {
        return Err(HpackError::InvalidStringLiteral(
            "string length exceeds available data".into(),
        ));
    }

    let raw = &src[consumed..consumed + length];
    let value = if huffman {
        Bytes::from(huffman_decode(raw).map_err(HpackError::InvalidHuffman)?)
    } else {
        Bytes::copy_from_slice(raw)
    };

    Ok((value, consumed + length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hpack::encoder::{Encoder, encode_integer};

    #[test]
    fn test_decode_integer_small() {
        let (value, consumed) = decode_integer(&[10], 5).unwrap();
        assert_eq!(value, 10);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_decode_integer_large() {
        // 1337 with 5-bit prefix
        let (value, consumed) = decode_integer(&[31, 154, 10], 5).unwrap();
        assert_eq!(value, 1337);
        assert_eq!(consumed, 3);
    }

    #[test]
    fn test_integer_roundtrip() {
        for val in [0, 1, 30, 31, 127, 128, 255, 1337, 65535, 1_000_000] {
            for prefix in [4, 5, 6, 7] {
                let mut buf = Vec::new();
                encode_integer(&mut buf, prefix, 0x00, val);
                let (decoded, consumed) = decode_integer(&buf, prefix).unwrap();
                assert_eq!(
                    decoded, val,
                    "roundtrip failed for val={} prefix={}",
                    val, prefix
                );
                assert_eq!(consumed, buf.len());
            }
        }
    }

    #[test]
    fn test_hpack_roundtrip() {
        let mut encoder = Encoder::new(4096);
        let mut decoder = Decoder::new(4096);

        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"GET"),
            (b":scheme", b"https"),
            (b":path", b"/index.html"),
            (b":authority", b"www.example.com"),
            (b"custom-key", b"custom-value"),
        ];

        let mut encoded = Vec::new();
        encoder.encode(headers.iter().copied(), &mut encoded);

        let decoded = decoder.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), headers.len());
        for (i, (name, value)) in headers.iter().enumerate() {
            assert_eq!(decoded[i].name, *name);
            assert_eq!(decoded[i].value, *value);
            assert!(!decoded[i].sensitive);
        }
    }

    #[test]
    fn test_decode_integer_too_many_continuation_bytes() {
        let mut buf = vec![0x1f];
        buf.extend(std::iter::repeat_n(0x80, 6));
        buf.push(0x00);
        let result = decode_integer(&buf, 5);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("too many continuation bytes"), "got: {}", err);
    }

    #[test]
    fn test_decode_integer_max_continuation_bytes_ok() {
        let buf = vec![0x1f, 0x81, 0x80, 0x80, 0x80, 0x01];
        let result = decode_integer(&buf, 5);
        assert!(result.is_ok());
    }

    #[test]
    fn test_size_update_after_header_rejected() {
        let mut decoder = Decoder::new(4096);
        let mut buf = vec![0x82];
        buf.push(0x3f);
        buf.push(0xe1);
        buf.push(0x07);
        let result = decoder.decode(&buf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("start of header block")
        );
    }

    #[test]
    fn test_size_update_exceeds_max() {
        let mut decoder = Decoder::new(4096);
        let mut buf = Vec::new();
        crate::hpack::encoder::encode_integer(&mut buf, 5, 0x20, 8192);
        let result = decoder.decode(&buf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds SETTINGS max")
        );
    }

    #[test]
    fn test_size_update_at_start_ok() {
        let mut decoder = Decoder::new(4096);
        let mut buf = Vec::new();
        crate::hpack::encoder::encode_integer(&mut buf, 5, 0x20, 2048);
        buf.push(0x82);
        let result = decoder.decode(&buf);
        assert!(result.is_ok());
    }

    #[test]
    fn test_hpack_multiple_requests() {
        let mut encoder = Encoder::new(4096);
        let mut decoder = Decoder::new(4096);

        let headers1: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"GET"),
            (b":path", b"/"),
            (b"host", b"example.com"),
        ];
        let mut buf1 = Vec::new();
        encoder.encode(headers1.iter().copied(), &mut buf1);
        let decoded1 = decoder.decode(&buf1).unwrap();
        assert_eq!(decoded1.len(), 3);

        let headers2: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"GET"),
            (b":path", b"/other"),
            (b"host", b"example.com"),
        ];
        let mut buf2 = Vec::new();
        encoder.encode(headers2.iter().copied(), &mut buf2);
        let decoded2 = decoder.decode(&buf2).unwrap();
        assert_eq!(decoded2.len(), 3);
        assert_eq!(&decoded2[0].name[..], b":method");
        assert_eq!(&decoded2[0].value[..], b"GET");
        assert_eq!(&decoded2[1].name[..], b":path");
        assert_eq!(&decoded2[1].value[..], b"/other");
        assert_eq!(&decoded2[2].name[..], b"host");
        assert_eq!(&decoded2[2].value[..], b"example.com");
    }

    #[test]
    fn test_never_indexed_sensitivity_flag() {
        // Never-indexed literal: 0001xxxx prefix
        // Encode: 0x10 | 0 (new name), then name "authorization", then value "secret"
        let mut buf = Vec::new();
        buf.push(0x10); // never-indexed, new name, index=0
        // Name: "authorization" (13 bytes, no Huffman)
        buf.push(13); // length
        buf.extend_from_slice(b"authorization");
        // Value: "secret" (6 bytes, no Huffman)
        buf.push(6);
        buf.extend_from_slice(b"secret");

        let mut decoder = Decoder::new(4096);
        let decoded = decoder.decode(&buf).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(&decoded[0].name[..], b"authorization");
        assert_eq!(&decoded[0].value[..], b"secret");
        assert!(decoded[0].sensitive, "never-indexed should be sensitive");
    }

    #[test]
    fn test_without_indexing_not_sensitive() {
        // Without-indexing literal: 0000xxxx prefix
        let mut buf = Vec::new();
        buf.push(0x00); // without indexing, new name, index=0
        buf.push(4);
        buf.extend_from_slice(b"host");
        buf.push(11);
        buf.extend_from_slice(b"example.com");

        let mut decoder = Decoder::new(4096);
        let decoded = decoder.decode(&buf).unwrap();
        assert_eq!(decoded.len(), 1);
        assert!(
            !decoded[0].sensitive,
            "without-indexing should NOT be sensitive"
        );
    }

    #[test]
    fn test_header_list_size_limit_enforced() {
        let mut decoder = Decoder::new(4096);
        // Each header costs name.len() + value.len() + 32
        // :method GET = 7 + 3 + 32 = 42 bytes
        decoder.set_max_header_list_size(41); // just under one header

        // Indexed :method GET (index 2)
        let buf = vec![0x82];
        let result = decoder.decode(&buf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("header list too large")
        );
    }

    #[test]
    fn test_header_list_size_limit_exact_fit() {
        let mut decoder = Decoder::new(4096);
        // :method GET = 7 + 3 + 32 = 42
        decoder.set_max_header_list_size(42);

        let buf = vec![0x82];
        let result = decoder.decode(&buf);
        assert!(result.is_ok());
    }

    #[test]
    fn test_header_list_size_unlimited_by_default() {
        let mut decoder = Decoder::new(4096);
        // Decode many headers without limit
        let mut encoder = Encoder::new(4096);
        let headers: Vec<(&[u8], &[u8])> = vec![
            (b":method", b"GET"),
            (b":path", b"/"),
            (b"host", b"example.com"),
            (b"accept", b"*/*"),
        ];
        let mut buf = Vec::new();
        encoder.encode(headers.iter().copied(), &mut buf);
        let result = decoder.decode(&buf);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 4);
    }

    // --- Security-focused tests ---

    #[test]
    fn test_hpack_invalid_index_beyond_table() {
        let mut decoder = Decoder::new(4096);
        // Indexed header reference with index far beyond static+dynamic table
        // Static table has 61 entries, dynamic is empty
        // For 7-bit prefix: if value < 127, single byte 0x80 | value
        let buf = vec![0x80 | 100]; // index 100, way beyond static table (61 entries)
        let err = decoder.decode(&buf).unwrap_err();
        assert!(
            matches!(err, crate::error::HpackError::InvalidTableIndex(_)),
            "expected InvalidTableIndex, got: {err}"
        );
    }

    #[test]
    fn test_hpack_many_headers_exceed_list_limit() {
        let mut decoder = Decoder::new(4096);
        // Each header costs name.len() + value.len() + 32 bytes per RFC 7541 §4.1
        // Header "a: b" costs 1 + 1 + 32 = 34 bytes
        // 3 headers = 102, 4 headers = 136
        decoder.set_max_header_list_size(120);

        // Encode 4 literal headers without indexing: each "a: b"
        let mut buf = Vec::new();
        for _ in 0..4 {
            buf.push(0x00); // literal without indexing, new name
            buf.push(1); // name length = 1
            buf.push(b'a'); // name
            buf.push(1); // value length = 1
            buf.push(b'b'); // value
        }

        let err = decoder.decode(&buf).unwrap_err();
        assert!(
            matches!(err, crate::error::HpackError::HeaderListTooLarge(_)),
            "expected HeaderListTooLarge, got: {err}"
        );
    }

    #[test]
    fn test_hpack_headers_exactly_at_list_limit() {
        let mut decoder = Decoder::new(4096);
        // 3 headers of "a: b" = 3 * 34 = 102 bytes
        decoder.set_max_header_list_size(102);

        let mut buf = Vec::new();
        for _ in 0..3 {
            buf.push(0x00);
            buf.push(1);
            buf.push(b'a');
            buf.push(1);
            buf.push(b'b');
        }

        let result = decoder.decode(&buf);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn test_hpack_truncated_header_block() {
        let mut decoder = Decoder::new(4096);
        // Literal header that declares string length 10 but only provides 3 bytes
        let mut buf = Vec::new();
        buf.push(0x00); // literal without indexing, new name
        buf.push(10); // name length = 10
        buf.extend_from_slice(b"abc"); // only 3 bytes, not 10

        let err = decoder.decode(&buf).unwrap_err();
        assert!(
            matches!(err, crate::error::HpackError::InvalidStringLiteral(_)),
            "expected InvalidStringLiteral, got: {err}"
        );
    }

    #[test]
    fn test_hpack_empty_header_name_and_value() {
        let mut decoder = Decoder::new(4096);
        // Literal header with 0-length name and 0-length value
        let buf = vec![
            0x00, // literal without indexing, new name
            0,    // name length = 0
            0,    // value length = 0
        ];

        let result = decoder.decode(&buf);
        // Empty names/values are valid at HPACK level (HTTP/2 pseudo-header validation
        // is higher layer)
        assert!(result.is_ok());
        let headers = result.unwrap();
        assert_eq!(headers.len(), 1);
        assert!(headers[0].name.is_empty());
        assert!(headers[0].value.is_empty());
    }

    #[test]
    fn test_hpack_table_size_boundary() {
        let mut decoder = Decoder::new(4096);
        // Table size update to exactly the max (4096) should succeed
        let mut buf = Vec::new();
        crate::hpack::encoder::encode_integer(&mut buf, 5, 0x20, 4096);
        buf.push(0x82); // indexed :method GET
        let result = decoder.decode(&buf);
        assert!(result.is_ok());

        // Table size update to max + 1 (4097) should fail
        let mut decoder2 = Decoder::new(4096);
        let mut buf2 = Vec::new();
        crate::hpack::encoder::encode_integer(&mut buf2, 5, 0x20, 4097);
        let err = decoder2.decode(&buf2).unwrap_err();
        assert!(
            matches!(err, crate::error::HpackError::TableSizeOverflow(_)),
            "expected TableSizeOverflow, got: {err}"
        );
    }

    #[test]
    fn test_hpack_invalid_huffman_eos_padding() {
        let mut decoder = Decoder::new(4096);
        // Huffman-encoded string with invalid padding (not all 1-bits)
        // The Huffman flag is bit 7 of the string length byte
        let buf = vec![
            0x00, // literal without indexing, new name
            0x82, // Huffman flag (0x80) | length 2
            0xFF, 0x00, // Second byte has 0x00 which is invalid padding
            0x01, // value length = 1
            b'x', // value
        ];

        let result = decoder.decode(&buf);
        // This should fail due to invalid Huffman decoding
        assert!(result.is_err());
    }
}
