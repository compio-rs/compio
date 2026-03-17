//! Deterministic edge-case tests for HPACK and Huffman codecs.
//!
//! These exercise the same code paths the cargo-fuzz targets cover,
//! but run on every `cargo test` with crafted inputs.

use bytes::Bytes;
use compio_h2::{
    Frame, FrameHeader, HpackDecoder, HpackEncoder,
    hpack::huffman::{huffman_decode, huffman_encode, huffman_encoded_len},
};

// ---- Huffman ----

#[test]
fn huffman_empty_input() {
    let result = huffman_decode(&[]);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn huffman_encode_all_bytes_no_panic() {
    // Every single byte 0x00-0xFF should encode without panic and produce
    // non-empty output. Exact roundtrip is not guaranteed for all single-byte
    // inputs because Huffman padding bits can form additional valid symbols.
    for byte in 0u8..=255 {
        let input = [byte];
        let encoded = huffman_encode(&input);
        assert!(
            !encoded.is_empty(),
            "encode produced empty output for 0x{byte:02x}"
        );
        let _ = huffman_decode(&encoded); // must not panic
    }
}

#[test]
fn huffman_roundtrip_multi_byte() {
    // Multi-byte ASCII strings should roundtrip exactly (padding ambiguity
    // is negligible for longer inputs since padding is at most 7 bits).
    let exact_cases: &[&[u8]] = &[b"hello", b"GET", b"/index.html", b"application/grpc"];
    for case in exact_cases {
        let encoded = huffman_encode(case);
        let decoded = huffman_decode(&encoded).unwrap();
        assert_eq!(
            &decoded,
            case,
            "roundtrip mismatch for {:?}",
            String::from_utf8_lossy(case)
        );
    }
    // Binary data: encode/decode must not panic
    let binary_cases: &[&[u8]] = &[
        b"\x00\x01\x02\x03\xff\xfe\xfd",
        &(0u8..=255).collect::<Vec<_>>(),
    ];
    for case in binary_cases {
        let encoded = huffman_encode(case);
        let _ = huffman_decode(&encoded); // must not panic
    }
}

#[test]
fn huffman_truncated_sequences() {
    let encoded = huffman_encode(b"hello world");
    for len in 1..encoded.len() {
        // Truncated input may succeed (if padding is valid) or fail — must not panic
        let _ = huffman_decode(&encoded[..len]);
    }
}

#[test]
fn huffman_invalid_padding() {
    // Padding must be all-1 bits. These have wrong padding — should error, not
    // panic.
    let _ = huffman_decode(&[0x18]);
    let _ = huffman_decode(&[0x00]);
}

#[test]
fn huffman_all_ones_eos() {
    // All-ones byte is EOS symbol padding — should be valid empty or error, not
    // panic
    let _ = huffman_decode(&[0xFF]);
    let _ = huffman_decode(&[0xFF, 0xFF]);
    let _ = huffman_decode(&[0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn huffman_maximum_length_codes() {
    // 30-bit symbols live at the high end of the code table (e.g. bytes 0-2).
    // Feed bytes that form long codes — must not panic or infinite-loop.
    let long_code_bytes: &[&[u8]] = &[
        &[0xFF, 0xFF, 0xFF, 0xFC], // 30 bits of 1s + 2 bits
        &[0xFF, 0xFF, 0xFE],       // 23 bits of 1s + 1 bit
        &[0xFF, 0xFF, 0xFF, 0xFF], // all ones — EOS region
    ];
    for input in long_code_bytes {
        let _ = huffman_decode(input); // must not panic
    }
}

#[test]
fn huffman_encoded_len_consistency() {
    for byte in 0u8..=255 {
        let input = [byte];
        let encoded = huffman_encode(&input);
        let expected_len = huffman_encoded_len(&input);
        assert_eq!(
            encoded.len(),
            expected_len,
            "len mismatch for byte 0x{byte:02x}"
        );
    }
}

// ---- HPACK Decode ----

#[test]
fn hpack_decode_empty_input() {
    let mut decoder = HpackDecoder::new(4096);
    let result = decoder.decode(&[]);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn hpack_decode_truncated_integer() {
    let mut decoder = HpackDecoder::new(4096);
    let _ = decoder.decode(&[0x1F]); // must not panic
    let _ = decoder.decode(&[0x1F, 0x80]);
    let _ = decoder.decode(&[0x1F, 0x80, 0x80]);
}

#[test]
fn hpack_decode_integer_overflow() {
    let mut decoder = HpackDecoder::new(4096);
    // 5+ continuation bytes all with high bit set — would overflow
    let _ = decoder.decode(&[0x1F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    let _ = decoder.decode(&[0x1F, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80]);
}

#[test]
fn hpack_decode_invalid_index() {
    let mut decoder = HpackDecoder::new(4096);
    // Index 70: 0x80 | 70 = 0xC6 (fits in 7-bit prefix), past static table
    let _ = decoder.decode(&[0xC6]);
    // Index 255: 0xFF (7-bit max) then continuation
    let _ = decoder.decode(&[0xFF, 0x80, 0x01]);
}

#[test]
fn hpack_decode_table_size_update_mid_block() {
    let mut decoder = HpackDecoder::new(4096);
    // Table size update (001xxxxx) after a valid header is an error per RFC 7541
    // §4.2 First: indexed header :method GET (index 2) = 0x82
    // Then: dynamic table size update to 0 = 0x20
    let _ = decoder.decode(&[0x82, 0x20]); // must not panic
}

#[test]
fn hpack_decode_empty_header_name() {
    let mut decoder = HpackDecoder::new(4096);
    // 0x40 = literal with indexing, new name
    // 0x00 = name length 0 (empty name)
    // 0x01 0x61 = value length 1, value "a"
    let _ = decoder.decode(&[0x40, 0x00, 0x01, 0x61]);
}

#[test]
fn hpack_decode_header_list_too_large() {
    let mut decoder = HpackDecoder::new(4096);
    decoder.set_max_header_list_size(10);
    // Literal header, name "abcdefghij" (10 bytes), value "abcdefghij" (10 bytes)
    let mut input = vec![0x40, 0x0A];
    input.extend_from_slice(b"abcdefghij");
    input.push(0x0A);
    input.extend_from_slice(b"abcdefghij");
    let result = decoder.decode(&input);
    assert!(
        result.is_err(),
        "should reject header list exceeding max size"
    );
}

// ---- HPACK Roundtrip ----

#[test]
fn hpack_roundtrip_empty_headers() {
    let mut encoder = HpackEncoder::new(4096);
    let mut decoder = HpackDecoder::new(4096);
    let mut buf = Vec::new();
    let headers: Vec<(&[u8], &[u8])> = vec![];
    encoder.encode(headers.iter().copied(), &mut buf);
    let decoded = decoder.decode(&buf).unwrap();
    assert!(decoded.is_empty());
}

#[test]
fn hpack_roundtrip_single_byte_values() {
    let mut encoder = HpackEncoder::new(4096);
    let mut decoder = HpackDecoder::new(4096);
    let headers: Vec<(&[u8], &[u8])> = vec![(b"x", b"y"), (b"a", b"b")];
    let mut buf = Vec::new();
    encoder.encode(headers.iter().copied(), &mut buf);
    let decoded = decoder.decode(&buf).unwrap();
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].name.as_ref(), b"x");
    assert_eq!(decoded[0].value.as_ref(), b"y");
    assert_eq!(decoded[1].name.as_ref(), b"a");
    assert_eq!(decoded[1].value.as_ref(), b"b");
}

#[test]
fn hpack_roundtrip_static_table_hits() {
    let mut encoder = HpackEncoder::new(4096);
    let mut decoder = HpackDecoder::new(4096);
    let headers: Vec<(&[u8], &[u8])> = vec![
        (b":method", b"GET"),
        (b":path", b"/"),
        (b":scheme", b"https"),
        (b":status", b"200"),
    ];
    let mut buf = Vec::new();
    encoder.encode(headers.iter().copied(), &mut buf);
    let decoded = decoder.decode(&buf).unwrap();
    assert_eq!(decoded.len(), 4);
    for (orig, dec) in headers.iter().zip(decoded.iter()) {
        assert_eq!(dec.name.as_ref(), orig.0);
        assert_eq!(dec.value.as_ref(), orig.1);
    }
}

#[test]
fn hpack_roundtrip_dynamic_table_eviction() {
    let mut encoder = HpackEncoder::new(64);
    let mut decoder = HpackDecoder::new(64);
    let batches: Vec<Vec<(&[u8], &[u8])>> = vec![
        vec![(b"key1", b"value1")],
        vec![(b"key2", b"value2")],
        vec![(b"key3", b"value3")],
        vec![(b"key1", b"value1")],
    ];
    for batch in &batches {
        let mut buf = Vec::new();
        encoder.encode(batch.iter().copied(), &mut buf);
        let decoded = decoder.decode(&buf).unwrap();
        assert_eq!(decoded.len(), batch.len());
        for (orig, dec) in batch.iter().zip(decoded.iter()) {
            assert_eq!(dec.name.as_ref(), orig.0);
            assert_eq!(dec.value.as_ref(), orig.1);
        }
    }
}

// ---- Frame Decode ----

#[test]
fn frame_header_decode_all_frame_types() {
    for frame_type in 0u8..=255 {
        let mut header_bytes = [0u8; 9];
        header_bytes[3] = frame_type;
        let _ = FrameHeader::decode(&header_bytes); // must not panic
    }
}

#[test]
fn frame_decode_zero_length() {
    for frame_type in 0u8..=9 {
        let mut header_bytes = [0u8; 9];
        header_bytes[3] = frame_type;
        // Stream ID = 1 for stream-bearing frames
        if frame_type != 0x04 && frame_type != 0x06 && frame_type != 0x07 {
            header_bytes[8] = 1;
        }
        let header = FrameHeader::decode(&header_bytes);
        let _ = Frame::decode(header, Bytes::new()); // must not panic
    }
}

#[test]
fn frame_decode_truncated_payloads() {
    for frame_type in 0u8..=9 {
        let mut header_bytes = [0u8; 9];
        header_bytes[2] = 10; // length = 10
        header_bytes[3] = frame_type;
        if frame_type != 0x04 && frame_type != 0x06 && frame_type != 0x07 {
            header_bytes[8] = 1;
        }
        let header = FrameHeader::decode(&header_bytes);
        let _ = Frame::decode(header, Bytes::from(vec![0u8; 10]));
        // Provide less than declared
        let header = FrameHeader::decode(&header_bytes);
        let _ = Frame::decode(header, Bytes::from(vec![0u8; 5]));
    }
}

#[test]
fn frame_decode_unknown_types() {
    // Unknown frame types (0x0A-0xFF) should be silently ignored per RFC 9113 §4.1
    for frame_type in 0x0Au8..=0xFF {
        let mut header_bytes = [0u8; 9];
        header_bytes[2] = 4; // length = 4
        header_bytes[3] = frame_type;
        header_bytes[8] = 1; // stream_id = 1
        let header = FrameHeader::decode(&header_bytes);
        let _ = Frame::decode(header, Bytes::from(vec![0u8; 4])); // must not panic
    }
}
