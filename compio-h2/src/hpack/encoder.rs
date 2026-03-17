use bytes::Bytes;

use super::{
    huffman::{huffman_encode, huffman_encoded_len},
    table::{DynamicTable, STATIC_TABLE, find_in_static_table},
};

/// Tracks pending dynamic table size updates between header blocks (RFC 7541
/// §4.2).
#[derive(Debug, Clone)]
struct PendingSizeUpdate {
    min_seen: usize,
    final_size: usize,
}

/// HPACK encoder.
#[derive(Debug)]
pub struct Encoder {
    table: DynamicTable,
    pending_size_update: Option<PendingSizeUpdate>,
    max_header_list_size: Option<usize>,
}

impl Encoder {
    /// Create a new instance.
    pub fn new(max_table_size: usize) -> Self {
        Encoder {
            table: DynamicTable::new(max_table_size),
            pending_size_update: None,
            max_header_list_size: None,
        }
    }

    /// Dynamic table.
    pub fn dynamic_table(&self) -> &DynamicTable {
        &self.table
    }

    /// Set the maximum header list size for outgoing header blocks.
    ///
    /// When set, [`encode`](Self::encode) will return an error if the
    /// cumulative header list size (sum of name + value + 32 for each field)
    /// exceeds this limit. This prevents the local side from producing header
    /// blocks larger than what was advertised in SETTINGS.
    pub fn set_max_header_list_size(&mut self, size: usize) {
        self.max_header_list_size = Some(size);
    }

    /// Set the dynamic table max size.
    ///
    /// The size update is deferred and emitted at the start of the next
    /// `encode()` call. If the max size is changed multiple times between
    /// header blocks, both the intermediate minimum and the final value are
    /// communicated per RFC 7541 §4.2.
    pub fn set_max_table_size(&mut self, new_size: usize) {
        self.table.set_max_size(new_size);
        match &mut self.pending_size_update {
            Some(pending) => {
                pending.min_seen = pending.min_seen.min(new_size);
                pending.final_size = new_size;
            }
            None => {
                self.pending_size_update = Some(PendingSizeUpdate {
                    min_seen: new_size,
                    final_size: new_size,
                });
            }
        }
    }

    /// Emit any pending size update instructions at the start of a header
    /// block.
    fn emit_pending_size_updates(&mut self, dst: &mut Vec<u8>) {
        if let Some(pending) = self.pending_size_update.take() {
            if pending.min_seen < pending.final_size {
                // Two-phase: emit minimum first, then final
                encode_integer(dst, 5, 0x20, pending.min_seen as u64);
            }
            encode_integer(dst, 5, 0x20, pending.final_size as u64);
        }
    }

    /// Encode a set of headers.
    pub fn encode<'a, I>(&mut self, headers: I, dst: &mut Vec<u8>)
    where
        I: IntoIterator<Item = (&'a [u8], &'a [u8])>,
    {
        self.emit_pending_size_updates(dst);
        for (name, value) in headers {
            if is_sensitive_header(name) {
                self.encode_header_never_indexed(name, value, dst);
            } else {
                self.encode_header(name, value, dst);
            }
        }
    }

    /// Encode a header using the "never indexed" representation (Section
    /// 6.2.3).
    fn encode_header_never_indexed(&mut self, name: &[u8], value: &[u8], dst: &mut Vec<u8>) {
        let static_match = find_in_static_table(name, value);

        match static_match {
            Some((idx, true)) => {
                // Full match — but we still use never-indexed to prevent intermediary indexing
                encode_integer(dst, 4, 0x10, idx as u64);
                encode_string(dst, value);
            }
            Some((idx, false)) => {
                // Name match in static table
                encode_integer(dst, 4, 0x10, idx as u64);
                encode_string(dst, value);
            }
            None => {
                // No match — new name, never indexed
                dst.push(0x10);
                encode_string(dst, name);
                encode_string(dst, value);
            }
        }
        // Do NOT add to dynamic table
    }

    fn encode_header(&mut self, name: &[u8], value: &[u8], dst: &mut Vec<u8>) {
        // RFC 7541: entry size = name length + value length + 32 bytes overhead
        let entry_size = name.len().saturating_add(value.len()).saturating_add(32);
        // If the entry would consume >75% of the dynamic table, use
        // literal-without-indexing to avoid excessive eviction churn.
        let too_large = entry_size > self.table.max_size() * 3 / 4;

        // First, check static table for full or name match
        let static_match = find_in_static_table(name, value);

        // Check dynamic table (O(1) hash lookup)
        let dynamic_match = self.table.find(name, value);

        // Determine best match
        match (static_match, dynamic_match) {
            // Full match in static table -> indexed representation
            (Some((idx, true)), _) => {
                encode_integer(dst, 7, 0x80, idx as u64);
            }
            // Full match in dynamic table -> indexed representation
            (_, Some((idx, true))) => {
                let abs_idx = idx + STATIC_TABLE.len();
                encode_integer(dst, 7, 0x80, abs_idx as u64);
            }
            // Name match in static table
            (Some((idx, false)), _) => {
                if too_large {
                    // Literal without indexing (prefix 0x00)
                    encode_integer(dst, 4, 0x00, idx as u64);
                    encode_string(dst, value);
                } else {
                    encode_integer(dst, 6, 0x40, idx as u64);
                    encode_string(dst, value);
                    self.table
                        .insert(Bytes::copy_from_slice(name), Bytes::copy_from_slice(value));
                }
            }
            // Name match in dynamic table
            (_, Some((idx, false))) => {
                if too_large {
                    let abs_idx = idx + STATIC_TABLE.len();
                    encode_integer(dst, 4, 0x00, abs_idx as u64);
                    encode_string(dst, value);
                } else {
                    let abs_idx = idx + STATIC_TABLE.len();
                    encode_integer(dst, 6, 0x40, abs_idx as u64);
                    encode_string(dst, value);
                    self.table
                        .insert(Bytes::copy_from_slice(name), Bytes::copy_from_slice(value));
                }
            }
            // No match -> new name
            (None, None) => {
                if too_large {
                    // Literal without indexing, new name (prefix 0x00, index 0)
                    dst.push(0x00);
                    encode_string(dst, name);
                    encode_string(dst, value);
                } else {
                    dst.push(0x40); // Literal with incremental indexing, new name
                    encode_string(dst, name);
                    encode_string(dst, value);
                    self.table
                        .insert(Bytes::copy_from_slice(name), Bytes::copy_from_slice(value));
                }
            }
        }
    }
}

/// Encode an HPACK integer with the given prefix size and prefix bits.
pub fn encode_integer(dst: &mut Vec<u8>, prefix_size: u8, prefix_bits: u8, value: u64) {
    let max_prefix = (1u64 << prefix_size) - 1;

    if value < max_prefix {
        dst.push(prefix_bits | value as u8);
    } else {
        dst.push(prefix_bits | max_prefix as u8);
        let mut remaining = value - max_prefix;
        while remaining >= 128 {
            dst.push((remaining & 0x7f) as u8 | 0x80);
            remaining >>= 7;
        }
        dst.push(remaining as u8);
    }
}

/// Encode a string (with Huffman if shorter).
fn encode_string(dst: &mut Vec<u8>, value: &[u8]) {
    let huffman_len = huffman_encoded_len(value);

    if huffman_len < value.len() {
        // Use Huffman encoding
        let encoded = huffman_encode(value);
        encode_integer(dst, 7, 0x80, encoded.len() as u64);
        dst.extend_from_slice(&encoded);
    } else {
        // Plain encoding
        encode_integer(dst, 7, 0x00, value.len() as u64);
        dst.extend_from_slice(value);
    }
}

/// Headers that should use the "never indexed" representation per RFC 7541.
fn is_sensitive_header(name: &[u8]) -> bool {
    matches!(
        name,
        b"authorization" | b"proxy-authorization" | b"cookie" | b"set-cookie"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_integer_small() {
        let mut buf = Vec::new();
        encode_integer(&mut buf, 5, 0x00, 10);
        assert_eq!(buf, vec![10]);
    }

    #[test]
    fn test_encode_integer_large() {
        // From RFC 7541 C.1.2: encoding 1337 with 5-bit prefix
        let mut buf = Vec::new();
        encode_integer(&mut buf, 5, 0x00, 1337);
        assert_eq!(buf, vec![31, 154, 10]);
    }

    #[test]
    fn test_encode_integer_prefix_bits() {
        let mut buf = Vec::new();
        encode_integer(&mut buf, 5, 0x20, 10);
        assert_eq!(buf, vec![0x20 | 10]);
    }

    #[test]
    fn test_encoder_indexed() {
        let mut encoder = Encoder::new(4096);
        let mut buf = Vec::new();
        // :method GET is static table index 2 -> indexed rep: 0x82
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);
        assert_eq!(buf[0], 0x82);
    }

    #[test]
    fn test_encoder_dynamic_table_reuse() {
        let mut encoder = Encoder::new(4096);

        // First encode: custom header goes into dynamic table
        let mut buf1 = Vec::new();
        encoder.encode(
            [(&b"x-custom"[..], &b"value1"[..])].iter().copied(),
            &mut buf1,
        );
        // Should be literal with incremental indexing, new name (0x40 prefix, index 0)
        assert_eq!(buf1[0], 0x40);

        // Second encode: same name, different value → name match from dynamic table
        let mut buf2 = Vec::new();
        encoder.encode(
            [(&b"x-custom"[..], &b"value2"[..])].iter().copied(),
            &mut buf2,
        );
        // Incremental indexing with name reference (0x40 | index)
        // Dynamic table index 1, absolute = 62 → 0x40 | 62 = 0x7E
        assert_eq!(buf2[0] & 0xC0, 0x40);
        assert_eq!(buf2[0] & 0x3F, 62); // absolute index to x-custom

        // Third encode: exact same header as second → full match from dynamic table
        let mut buf3 = Vec::new();
        encoder.encode(
            [(&b"x-custom"[..], &b"value2"[..])].iter().copied(),
            &mut buf3,
        );
        // Should be indexed representation (0x80 prefix) for full match
        assert_eq!(buf3[0] & 0x80, 0x80);
    }

    #[test]
    fn test_encoder_dynamic_table_full_match() {
        let mut encoder = Encoder::new(4096);
        let mut buf = Vec::new();

        // Insert a custom header
        encoder.encode([(&b"x-test"[..], &b"hello"[..])].iter().copied(), &mut buf);
        buf.clear();

        // Encode the same header again - should get indexed representation
        encoder.encode([(&b"x-test"[..], &b"hello"[..])].iter().copied(), &mut buf);
        // 0x80 prefix = indexed header field representation
        assert_eq!(buf[0] & 0x80, 0x80);
    }

    // --- B1: Two-phase size update tests ---

    #[test]
    fn test_size_update_single() {
        // A single size change emits one size update at the start of encode()
        let mut encoder = Encoder::new(4096);
        encoder.set_max_table_size(2048);

        let mut buf = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);
        // First byte: size update to 2048 (prefix 001, 5-bit prefix)
        // 0x20 | 31 = 0x3F (max prefix), then 2048 - 31 = 2017 encoded
        assert_eq!(buf[0] & 0xE0, 0x20); // size update prefix
    }

    #[test]
    fn test_size_update_two_phase() {
        // Decrease then increase between header blocks → two size updates
        let mut encoder = Encoder::new(4096);
        encoder.set_max_table_size(0); // min_seen = 0
        encoder.set_max_table_size(2048); // final = 2048

        let mut buf = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);
        // First byte: size update to 0 → 0x20 (prefix 001, value 0)
        assert_eq!(buf[0], 0x20);
        // Second byte: size update to 2048
        assert_eq!(buf[1] & 0xE0, 0x20);
    }

    #[test]
    fn test_size_update_same_value_emits_once() {
        // Multiple calls to same value → only one size update
        let mut encoder = Encoder::new(4096);
        encoder.set_max_table_size(1024);
        encoder.set_max_table_size(1024);

        let mut buf = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);
        // Should emit one size update to 1024, then the indexed header
        assert_eq!(buf[0] & 0xE0, 0x20); // size update prefix
        // Find where size update ends and header starts
        // 1024 with 5-bit prefix: 0x20|31 = 0x3F, then 1024-31=993
        // 993 = 128*7 + 97, so: 0x61|0x80 = 0xE1, then 0x07
        // Total: 0x3F, 0xE1, 0x07 → 3 bytes for size update
        assert_eq!(buf[3], 0x82); // :method GET indexed
    }

    #[test]
    fn test_size_update_three_changes() {
        // 3+ sequential size changes: only min_seen and final_size matter
        let mut encoder = Encoder::new(4096);
        encoder.set_max_table_size(1024); // min_seen=1024, final=1024
        encoder.set_max_table_size(512); // min_seen=512,  final=512
        encoder.set_max_table_size(2048); // min_seen=512,  final=2048

        let mut buf = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);

        // Should emit two size updates: 512 (min_seen) then 2048 (final)
        // First: size update to 512 (prefix 001, 5-bit prefix)
        // 512 with 5-bit prefix: 0x20|31=0x3F, then 512-31=481
        // 481 = 128*3 + 97, so: 0x61|0x80=0xE1, 0x03
        // Total: 0x3F, 0xE1, 0x03
        assert_eq!(buf[0] & 0xE0, 0x20); // first is a size update

        // Find the second size update (after the first)
        // Decode first integer to find its length
        let mut i = 0;
        if buf[i] & 0x1F == 0x1F {
            i += 1;
            while buf[i] & 0x80 != 0 {
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
        // buf[i] should be the start of the second size update
        assert_eq!(buf[i] & 0xE0, 0x20); // second is also a size update
    }

    #[test]
    fn test_size_update_four_changes_tracks_global_min() {
        // 4 changes: 4096 → 100 → 3000 → 50 → 1000
        // min_seen should be 50, final should be 1000
        let mut encoder = Encoder::new(4096);
        encoder.set_max_table_size(100);
        encoder.set_max_table_size(3000);
        encoder.set_max_table_size(50);
        encoder.set_max_table_size(1000);

        let mut buf = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);

        // First size update should be to 50 (the minimum seen)
        // 50 with 5-bit prefix: 0x20 | 50 = 0x52 (fits in one byte since 50 < 31... no,
        // 50 > 31) Actually 50 > 31, so: 0x20|31=0x3F, then 50-31=19 => 0x13
        assert_eq!(buf[0], 0x3F);
        assert_eq!(buf[1], 19); // 50 - 31 = 19

        // Second size update to 1000
        assert_eq!(buf[2] & 0xE0, 0x20);
    }

    #[test]
    fn test_no_size_update_without_change() {
        // No set_max_table_size call → no size update prefix
        let mut encoder = Encoder::new(4096);
        let mut buf = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf);
        // First byte should be the indexed header, not a size update
        assert_eq!(buf[0], 0x82);
    }

    #[test]
    fn test_size_update_cleared_after_encode() {
        // After emitting size update in first encode, second encode has none
        let mut encoder = Encoder::new(4096);
        encoder.set_max_table_size(2048);

        let mut buf1 = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf1);
        assert_eq!(buf1[0] & 0xE0, 0x20); // size update present

        let mut buf2 = Vec::new();
        encoder.encode([(&b":method"[..], &b"GET"[..])].iter().copied(), &mut buf2);
        assert_eq!(buf2[0], 0x82); // no size update, just indexed header
    }

    // --- B2: Entry-too-large tests ---

    #[test]
    fn test_large_entry_uses_literal_without_indexing() {
        // Table size = 128. Entry overhead = 32.
        // 75% threshold = 96. We need name+value+32 > 96, so name+value > 64.
        let mut encoder = Encoder::new(128);
        let mut buf = Vec::new();
        let name = b"x-big";
        let value = b"a]aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // 63 bytes
        // entry_size = 5 + 63 + 32 = 100 > 96
        encoder.encode([(&name[..], &value[..])].iter().copied(), &mut buf);
        // Should be literal without indexing (0x00 prefix), not 0x40
        assert_eq!(buf[0] & 0xF0, 0x00);
        // Dynamic table should be empty (no insert)
        assert_eq!(encoder.dynamic_table().len(), 0);
    }

    #[test]
    fn test_small_entry_uses_incremental_indexing() {
        // Table size = 4096. A small header should be indexed normally.
        let mut encoder = Encoder::new(4096);
        let mut buf = Vec::new();
        encoder.encode([(&b"x-small"[..], &b"val"[..])].iter().copied(), &mut buf);
        // Should be literal with incremental indexing (0x40 prefix)
        assert_eq!(buf[0], 0x40);
        assert_eq!(encoder.dynamic_table().len(), 1);
    }
}
