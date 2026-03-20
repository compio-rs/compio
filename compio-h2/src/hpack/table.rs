use std::{collections::VecDeque, hash::Hasher};

use bytes::Bytes;
use fnv::FnvHashMap;

/// Static table from RFC 7541 Appendix A.
/// Index is 1-based in the spec; stored 0-based here.
pub static STATIC_TABLE: &[(&str, &str)] = &[
    // Index 1
    (":authority", ""),
    // Index 2
    (":method", "GET"),
    // Index 3
    (":method", "POST"),
    // Index 4
    (":path", "/"),
    // Index 5
    (":path", "/index.html"),
    // Index 6
    (":scheme", "http"),
    // Index 7
    (":scheme", "https"),
    // Index 8
    (":status", "200"),
    // Index 9
    (":status", "204"),
    // Index 10
    (":status", "206"),
    // Index 11
    (":status", "304"),
    // Index 12
    (":status", "400"),
    // Index 13
    (":status", "404"),
    // Index 14
    (":status", "500"),
    // Index 15
    ("accept-charset", ""),
    // Index 16
    ("accept-encoding", "gzip, deflate"),
    // Index 17
    ("accept-language", ""),
    // Index 18
    ("accept-ranges", ""),
    // Index 19
    ("accept", ""),
    // Index 20
    ("access-control-allow-origin", ""),
    // Index 21
    ("age", ""),
    // Index 22
    ("allow", ""),
    // Index 23
    ("authorization", ""),
    // Index 24
    ("cache-control", ""),
    // Index 25
    ("content-disposition", ""),
    // Index 26
    ("content-encoding", ""),
    // Index 27
    ("content-language", ""),
    // Index 28
    ("content-length", ""),
    // Index 29
    ("content-location", ""),
    // Index 30
    ("content-range", ""),
    // Index 31
    ("content-type", ""),
    // Index 32
    ("cookie", ""),
    // Index 33
    ("date", ""),
    // Index 34
    ("etag", ""),
    // Index 35
    ("expect", ""),
    // Index 36
    ("expires", ""),
    // Index 37
    ("from", ""),
    // Index 38
    ("host", ""),
    // Index 39
    ("if-match", ""),
    // Index 40
    ("if-modified-since", ""),
    // Index 41
    ("if-none-match", ""),
    // Index 42
    ("if-range", ""),
    // Index 43
    ("if-unmodified-since", ""),
    // Index 44
    ("last-modified", ""),
    // Index 45
    ("link", ""),
    // Index 46
    ("location", ""),
    // Index 47
    ("max-forwards", ""),
    // Index 48
    ("proxy-authenticate", ""),
    // Index 49
    ("proxy-authorization", ""),
    // Index 50
    ("range", ""),
    // Index 51
    ("referer", ""),
    // Index 52
    ("refresh", ""),
    // Index 53
    ("retry-after", ""),
    // Index 54
    ("server", ""),
    // Index 55
    ("set-cookie", ""),
    // Index 56
    ("strict-transport-security", ""),
    // Index 57
    ("transfer-encoding", ""),
    // Index 58
    ("user-agent", ""),
    // Index 59
    ("vary", ""),
    // Index 60
    ("via", ""),
    // Index 61
    ("www-authenticate", ""),
];

// Generated length-bucketed static table lookup (replaces LazyLock<HashMap>
// approach).
include!(concat!(env!("OUT_DIR"), "/static_table_lookup.rs"));

/// Entry overhead per RFC 7541 Section 4.1.
const ENTRY_OVERHEAD: usize = 32;

/// Compute FNV-1a hash of a (name, value) pair without allocating.
fn hash_name_value(name: &[u8], value: &[u8]) -> u64 {
    let mut hasher = fnv::FnvHasher::default();
    hasher.write(name);
    hasher.write_u8(0xFF); // separator to avoid collisions between ("ab","c") and ("a","bc")
    hasher.write(value);
    hasher.finish()
}

/// Compute FNV-1a hash of a name without allocating.
fn hash_name(name: &[u8]) -> u64 {
    let mut hasher = fnv::FnvHasher::default();
    hasher.write(name);
    hasher.finish()
}

/// Dynamic table for HPACK with O(1) hash-based lookups.
///
/// Uses a sequence-number scheme: each inserted entry gets a monotonically
/// increasing sequence number. The 1-based HPACK dynamic index for an entry
/// with sequence `s` is `insert_count - s`. Hash maps store sequence numbers
/// keyed by pre-computed FNV hashes (zero allocation on lookup).
#[derive(Debug, Clone)]
pub struct DynamicTable {
    entries: VecDeque<(Bytes, Bytes)>,
    max_size: usize,
    current_size: usize,
    insert_count: usize,
    /// Maps hash(name, value) → sequence number. On collision, verify against
    /// entries.
    name_value_map: FnvHashMap<u64, usize>,
    /// Maps hash(name) → sequence number. On collision, verify against entries.
    name_map: FnvHashMap<u64, usize>,
}

impl DynamicTable {
    /// Create a new empty dynamic table with the given maximum size in bytes.
    pub fn new(max_size: usize) -> Self {
        DynamicTable {
            entries: VecDeque::new(),
            max_size,
            current_size: 0,
            insert_count: 0,
            name_value_map: FnvHashMap::with_hasher(Default::default()),
            name_map: FnvHashMap::with_hasher(Default::default()),
        }
    }

    /// Insert a new entry at the front of the table, evicting as needed.
    pub fn insert(&mut self, name: Bytes, value: Bytes) {
        let entry_size = name.len() + value.len() + ENTRY_OVERHEAD;

        // Evict entries until there's room
        while self.current_size + entry_size > self.max_size {
            if let Some((n, v)) = self.entries.pop_back() {
                self.remove_from_index(&n, &v);
                self.current_size -= n.len() + v.len() + ENTRY_OVERHEAD;
            } else {
                break;
            }
        }

        // If the entry itself exceeds max size, don't add it (but table is now empty)
        if entry_size <= self.max_size {
            let seq = self.insert_count;
            self.insert_count += 1;
            self.current_size += entry_size;

            let nv_hash = hash_name_value(&name, &value);
            let n_hash = hash_name(&name);
            self.name_value_map.insert(nv_hash, seq);
            self.name_map.insert(n_hash, seq);

            self.entries.push_front((name, value));
        } else {
            self.insert_count += 1;
        }
    }

    /// Remove an evicted entry from the hash indices.
    fn remove_from_index(&mut self, name: &Bytes, value: &Bytes) {
        // After pop_back, entries.len() is already decremented.
        // The evicted entry's seq = insert_count - old_len = insert_count -
        // (entries.len() + 1)
        let evicted_seq = self.insert_count - self.entries.len() - 1;

        let nv_hash = hash_name_value(name, value);
        if let Some(&seq) = self.name_value_map.get(&nv_hash)
            && seq == evicted_seq
        {
            self.name_value_map.remove(&nv_hash);
        }

        let n_hash = hash_name(name);
        if let Some(&seq) = self.name_map.get(&n_hash)
            && seq == evicted_seq
        {
            self.name_map.remove(&n_hash);
        }
    }

    /// O(1) lookup in the dynamic table.
    /// Returns (1-based dynamic index, full_match).
    pub fn find(&self, name: &[u8], value: &[u8]) -> Option<(usize, bool)> {
        // Check for full (name, value) match first — zero allocations
        let nv_hash = hash_name_value(name, value);
        if let Some(&seq) = self.name_value_map.get(&nv_hash) {
            let idx = self.insert_count - seq;
            if idx <= self.entries.len() {
                // Verify against actual entry to handle hash collisions
                if let Some((n, v)) = self.entries.get(idx - 1)
                    && n.as_ref() == name
                    && v.as_ref() == value
                {
                    return Some((idx, true));
                }
            }
        }

        // Check for name-only match — zero allocations
        let n_hash = hash_name(name);
        if let Some(&seq) = self.name_map.get(&n_hash) {
            let idx = self.insert_count - seq;
            if idx <= self.entries.len() {
                // Verify name matches
                if let Some((n, _)) = self.entries.get(idx - 1)
                    && n.as_ref() == name
                {
                    return Some((idx, false));
                }
            }
        }

        None
    }

    /// Get an entry by 0-based dynamic table index.
    pub fn get(&self, index: usize) -> Option<&(Bytes, Bytes)> {
        self.entries.get(index)
    }

    /// The number of entries in the dynamic table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the dynamic table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The current size of the dynamic table in bytes.
    pub fn current_size(&self) -> usize {
        self.current_size
    }

    /// The maximum allowed size of the dynamic table in bytes.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Set a new maximum table size, evicting entries as needed.
    pub fn set_max_size(&mut self, new_max: usize) {
        self.max_size = new_max;
        while self.current_size > self.max_size {
            if let Some((n, v)) = self.entries.pop_back() {
                self.remove_from_index(&n, &v);
                self.current_size -= n.len() + v.len() + ENTRY_OVERHEAD;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_table_size() {
        assert_eq!(STATIC_TABLE.len(), 61);
    }

    #[test]
    fn test_static_table_lookups() {
        // Full match: :method GET is index 2
        let result = find_in_static_table(b":method", b"GET");
        assert_eq!(result, Some((2, true)));

        // Full match: :method POST is index 3
        let result = find_in_static_table(b":method", b"POST");
        assert_eq!(result, Some((3, true)));

        // Name-only match: :authority with some value
        let result = find_in_static_table(b":authority", b"example.com");
        assert_eq!(result, Some((1, false)));

        // No match
        let result = find_in_static_table(b"x-custom", b"value");
        assert_eq!(result, None);
    }

    #[test]
    fn test_static_table_name_only_returns_first_index() {
        // :status has entries at indices 8-14, name-only should return 8
        let result = find_in_static_table(b":status", b"999");
        assert_eq!(result, Some((8, false)));

        // :method has entries at indices 2-3, name-only should return 2
        let result = find_in_static_table(b":method", b"PUT");
        assert_eq!(result, Some((2, false)));
    }

    #[test]
    fn test_dynamic_table_insert_and_get() {
        let mut table = DynamicTable::new(4096);
        table.insert(
            Bytes::from_static(b"custom-key"),
            Bytes::from_static(b"custom-value"),
        );

        assert_eq!(table.len(), 1);
        let entry = table.get(0).unwrap();
        assert_eq!(entry.0.as_ref(), b"custom-key");
        assert_eq!(entry.1.as_ref(), b"custom-value");
    }

    #[test]
    fn test_dynamic_table_eviction() {
        let mut table = DynamicTable::new(64);
        table.insert(Bytes::from_static(b"aaa"), Bytes::from_static(b"bbb"));
        assert_eq!(table.len(), 1);

        table.insert(Bytes::from_static(b"ccc"), Bytes::from_static(b"ddd"));
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(0).unwrap().0.as_ref(), b"ccc");
    }

    #[test]
    fn test_dynamic_table_set_max_size() {
        let mut table = DynamicTable::new(4096);
        table.insert(Bytes::from_static(b"key1"), Bytes::from_static(b"value1"));
        table.insert(Bytes::from_static(b"key2"), Bytes::from_static(b"value2"));
        assert_eq!(table.len(), 2);

        table.set_max_size(0);
        assert_eq!(table.len(), 0);
        assert_eq!(table.current_size(), 0);
    }

    #[test]
    fn test_dynamic_table_hash_lookup_full_match() {
        let mut table = DynamicTable::new(4096);
        table.insert(Bytes::from_static(b"key1"), Bytes::from_static(b"val1"));
        table.insert(Bytes::from_static(b"key2"), Bytes::from_static(b"val2"));

        assert_eq!(table.find(b"key2", b"val2"), Some((1, true)));
        assert_eq!(table.find(b"key1", b"val1"), Some((2, true)));
    }

    #[test]
    fn test_dynamic_table_hash_lookup_name_only() {
        let mut table = DynamicTable::new(4096);
        table.insert(Bytes::from_static(b"key1"), Bytes::from_static(b"val1"));
        table.insert(Bytes::from_static(b"key1"), Bytes::from_static(b"val2"));

        // Name-only match returns the most recent (index 1)
        assert_eq!(table.find(b"key1", b"val3"), Some((1, false)));
        // Full match for the newest
        assert_eq!(table.find(b"key1", b"val2"), Some((1, true)));
        // Full match for the older one
        assert_eq!(table.find(b"key1", b"val1"), Some((2, true)));
    }

    #[test]
    fn test_dynamic_table_hash_lookup_no_match() {
        let mut table = DynamicTable::new(4096);
        table.insert(Bytes::from_static(b"key1"), Bytes::from_static(b"val1"));

        assert_eq!(table.find(b"nonexistent", b"val"), None);
    }

    #[test]
    fn test_dynamic_table_hash_eviction_cleans_index() {
        let mut table = DynamicTable::new(64);
        table.insert(Bytes::from_static(b"aaa"), Bytes::from_static(b"bbb"));
        assert_eq!(table.find(b"aaa", b"bbb"), Some((1, true)));

        table.insert(Bytes::from_static(b"ccc"), Bytes::from_static(b"ddd"));
        assert_eq!(table.find(b"aaa", b"bbb"), None);
        assert_eq!(table.find(b"ccc", b"ddd"), Some((1, true)));
    }

    #[test]
    fn test_dynamic_table_set_max_size_cleans_index() {
        let mut table = DynamicTable::new(4096);
        table.insert(Bytes::from_static(b"key1"), Bytes::from_static(b"val1"));
        table.insert(Bytes::from_static(b"key2"), Bytes::from_static(b"val2"));

        table.set_max_size(0);
        assert_eq!(table.find(b"key1", b"val1"), None);
        assert_eq!(table.find(b"key2", b"val2"), None);
    }

    #[test]
    fn test_dynamic_table_duplicate_name_eviction() {
        // Each entry = 3 ("key") + 2 ("v1") + 32 = 37 bytes. Table fits exactly 2.
        let mut table = DynamicTable::new(74);
        table.insert(Bytes::from_static(b"key"), Bytes::from_static(b"v1"));
        table.insert(Bytes::from_static(b"key"), Bytes::from_static(b"v2"));

        assert_eq!(table.find(b"key", b"v1"), Some((2, true)));
        assert_eq!(table.find(b"key", b"v2"), Some((1, true)));

        // Insert a third, evicting v1
        table.insert(Bytes::from_static(b"key"), Bytes::from_static(b"v3"));
        // v1 full match is gone, but name "key" still matches (v3 is most recent)
        assert_eq!(table.find(b"key", b"v1"), Some((1, false)));
        assert_eq!(table.find(b"key", b"v3"), Some((1, true)));
        assert_eq!(table.find(b"key", b"v2"), Some((2, true)));
    }

    #[test]
    fn test_hash_name_value_separator_prevents_prefix_collisions() {
        // The 0xFF separator byte must ensure that different name/value splits
        // of the same concatenated bytes produce distinct hashes.
        assert_ne!(hash_name_value(b"ab", b"c"), hash_name_value(b"a", b"bc"),);
        assert_ne!(
            hash_name_value(b"content-type", b"text"),
            hash_name_value(b"content-typ", b"etext"),
        );
    }

    #[test]
    fn test_static_table_exhaustive_round_trip() {
        // Every entry in STATIC_TABLE must produce a full match at the correct
        // 1-based index via find_in_static_table.
        for (i, &(name, value)) in STATIC_TABLE.iter().enumerate() {
            let idx_1based = i + 1;
            let result = find_in_static_table(name.as_bytes(), value.as_bytes());
            assert_eq!(
                result,
                Some((idx_1based, true)),
                "full match failed for entry {idx_1based}: ({name:?}, {value:?})"
            );
        }
    }

    #[test]
    fn test_static_table_name_only_all_unique_names() {
        // For every unique name in the static table, looking up with a value
        // that does NOT appear in any entry should yield a name-only match at
        // the first 1-based index for that name.
        let mut seen = std::collections::HashMap::new();
        for (i, &(name, _)) in STATIC_TABLE.iter().enumerate() {
            seen.entry(name).or_insert(i + 1);
        }
        for (&name, &first_idx) in &seen {
            let result = find_in_static_table(name.as_bytes(), b"\xff\xfe\xfd");
            assert_eq!(
                result,
                Some((first_idx, false)),
                "name-only match failed for {name:?} (expected first index {first_idx})"
            );
        }
    }

    #[test]
    fn test_static_table_no_match_for_unknown_names() {
        assert_eq!(find_in_static_table(b"x-custom", b"val"), None);
        assert_eq!(find_in_static_table(b"", b""), None);
        assert_eq!(find_in_static_table(b"a", b""), None);
        assert_eq!(find_in_static_table(b"accept-charsets", b""), None); // extra 's'
    }
}
