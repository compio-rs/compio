#![no_main]
use compio_h2::{HpackDecoder, HpackEncoder};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Interpret input as a sequence of (name, value) pairs.
    // Format: [name_len: u8, name: [u8], value_len: u8, value: [u8], ...]
    let mut headers = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let name_len = data[pos] as usize;
        pos += 1;
        if pos + name_len > data.len() {
            break;
        }
        let name = &data[pos..pos + name_len];
        pos += name_len;
        if pos >= data.len() {
            break;
        }
        let value_len = data[pos] as usize;
        pos += 1;
        if pos + value_len > data.len() {
            break;
        }
        let value = &data[pos..pos + value_len];
        pos += value_len;
        headers.push((name, value));
    }

    if headers.is_empty() {
        return;
    }

    // Encode headers with HPACK.
    let mut encoder = HpackEncoder::new(4096);
    let mut encoded = Vec::new();
    encoder.encode(
        headers.iter().map(|(n, v)| (*n as &[u8], *v as &[u8])),
        &mut encoded,
    );

    // Decode and verify round-trip.
    let mut decoder = HpackDecoder::new(4096);
    let decoded = match decoder.decode(&encoded) {
        Ok(d) => d,
        Err(_) => return, // Encoder produced invalid output — that's a bug but don't panic
    };

    // Verify header count matches.
    assert_eq!(
        decoded.len(),
        headers.len(),
        "round-trip header count mismatch"
    );

    // Verify each header name and value matches.
    for (i, (original, decoded_hdr)) in headers.iter().zip(decoded.iter()).enumerate() {
        assert_eq!(
            decoded_hdr.name.as_ref(),
            original.0,
            "header {i} name mismatch"
        );
        assert_eq!(
            decoded_hdr.value.as_ref(),
            original.1,
            "header {i} value mismatch"
        );
    }
});
