use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::Entry},
    env, fs,
    io::Write,
    path::Path,
};

/// Huffman code table from RFC 7541 Appendix B.
/// Each entry is (code, bit_length).
static HUFFMAN_TABLE: [(u32, u8); 257] = [
    (0x1ff8, 13),
    (0x7fffd8, 23),
    (0xfffffe2, 28),
    (0xfffffe3, 28),
    (0xfffffe4, 28),
    (0xfffffe5, 28),
    (0xfffffe6, 28),
    (0xfffffe7, 28),
    (0xfffffe8, 28),
    (0xffffea, 24),
    (0x3ffffffc, 30),
    (0xfffffe9, 28),
    (0xfffffea, 28),
    (0x3ffffffd, 30),
    (0xfffffeb, 28),
    (0xfffffec, 28),
    (0xfffffed, 28),
    (0xfffffee, 28),
    (0xfffffef, 28),
    (0xffffff0, 28),
    (0xffffff1, 28),
    (0xffffff2, 28),
    (0x3ffffffe, 30),
    (0xffffff3, 28),
    (0xffffff4, 28),
    (0xffffff5, 28),
    (0xffffff6, 28),
    (0xffffff7, 28),
    (0xffffff8, 28),
    (0xffffff9, 28),
    (0xffffffa, 28),
    (0xffffffb, 28),
    (0x14, 6),
    (0x3f8, 10),
    (0x3f9, 10),
    (0xffa, 12),
    (0x1ff9, 13),
    (0x15, 6),
    (0xf8, 8),
    (0x7fa, 11),
    (0x3fa, 10),
    (0x3fb, 10),
    (0xf9, 8),
    (0x7fb, 11),
    (0xfa, 8),
    (0x16, 6),
    (0x17, 6),
    (0x18, 6),
    (0x0, 5),
    (0x1, 5),
    (0x2, 5),
    (0x19, 6),
    (0x1a, 6),
    (0x1b, 6),
    (0x1c, 6),
    (0x1d, 6),
    (0x1e, 6),
    (0x1f, 6),
    (0x5c, 7),
    (0xfb, 8),
    (0x7ffc, 15),
    (0x20, 6),
    (0xffb, 12),
    (0x3fc, 10),
    (0x1ffa, 13),
    (0x21, 6),
    (0x5d, 7),
    (0x5e, 7),
    (0x5f, 7),
    (0x60, 7),
    (0x61, 7),
    (0x62, 7),
    (0x63, 7),
    (0x64, 7),
    (0x65, 7),
    (0x66, 7),
    (0x67, 7),
    (0x68, 7),
    (0x69, 7),
    (0x6a, 7),
    (0x6b, 7),
    (0x6c, 7),
    (0x6d, 7),
    (0x6e, 7),
    (0x6f, 7),
    (0x70, 7),
    (0x71, 7),
    (0x72, 7),
    (0xfc, 8),
    (0x73, 7),
    (0xfd, 8),
    (0x1ffb, 13),
    (0x7fff0, 19),
    (0x1ffc, 13),
    (0x3ffc, 14),
    (0x22, 6),
    (0x7ffd, 15),
    (0x3, 5),
    (0x23, 6),
    (0x4, 5),
    (0x24, 6),
    (0x5, 5),
    (0x25, 6),
    (0x26, 6),
    (0x27, 6),
    (0x6, 5),
    (0x74, 7),
    (0x75, 7),
    (0x28, 6),
    (0x29, 6),
    (0x2a, 6),
    (0x7, 5),
    (0x2b, 6),
    (0x76, 7),
    (0x2c, 6),
    (0x8, 5),
    (0x9, 5),
    (0x2d, 6),
    (0x77, 7),
    (0x78, 7),
    (0x79, 7),
    (0x7a, 7),
    (0x7b, 7),
    (0x7ffe, 15),
    (0x7fc, 11),
    (0x3ffd, 14),
    (0x1ffd, 13),
    (0xffffffc, 28),
    (0xfffe6, 20),
    (0x3fffd2, 22),
    (0xfffe7, 20),
    (0xfffe8, 20),
    (0x3fffd3, 22),
    (0x3fffd4, 22),
    (0x3fffd5, 22),
    (0x7fffd9, 23),
    (0x3fffd6, 22),
    (0x7fffda, 23),
    (0x7fffdb, 23),
    (0x7fffdc, 23),
    (0x7fffdd, 23),
    (0x7fffde, 23),
    (0xffffeb, 24),
    (0x7fffdf, 23),
    (0xffffec, 24),
    (0xffffed, 24),
    (0x3fffd7, 22),
    (0x7fffe0, 23),
    (0xffffee, 24),
    (0x7fffe1, 23),
    (0x7fffe2, 23),
    (0x7fffe3, 23),
    (0x7fffe4, 23),
    (0x1fffdc, 21),
    (0x3fffd8, 22),
    (0x7fffe5, 23),
    (0x3fffd9, 22),
    (0x7fffe6, 23),
    (0x7fffe7, 23),
    (0xffffef, 24),
    (0x3fffda, 22),
    (0x1fffdd, 21),
    (0xfffe9, 20),
    (0x3fffdb, 22),
    (0x3fffdc, 22),
    (0x7fffe8, 23),
    (0x7fffe9, 23),
    (0x1fffde, 21),
    (0x7fffea, 23),
    (0x3fffdd, 22),
    (0x3fffde, 22),
    (0xfffff0, 24),
    (0x1fffdf, 21),
    (0x3fffdf, 22),
    (0x7fffeb, 23),
    (0x7fffec, 23),
    (0x1fffe0, 21),
    (0x1fffe1, 21),
    (0x3fffe0, 22),
    (0x1fffe2, 21),
    (0x7fffed, 23),
    (0x3fffe1, 22),
    (0x7fffee, 23),
    (0x7fffef, 23),
    (0xfffea, 20),
    (0x3fffe2, 22),
    (0x3fffe3, 22),
    (0x3fffe4, 22),
    (0x7ffff0, 23),
    (0x3fffe5, 22),
    (0x3fffe6, 22),
    (0x7ffff1, 23),
    (0x3ffffe0, 26),
    (0x3ffffe1, 26),
    (0xfffeb, 20),
    (0x7fff1, 19),
    (0x3fffe7, 22),
    (0x7ffff2, 23),
    (0x3fffe8, 22),
    (0x1ffffec, 25),
    (0x3ffffe2, 26),
    (0x3ffffe3, 26),
    (0x3ffffe4, 26),
    (0x7ffffde, 27),
    (0x7ffffdf, 27),
    (0x3ffffe5, 26),
    (0xfffff1, 24),
    (0x1ffffed, 25),
    (0x7fff2, 19),
    (0x1fffe3, 21),
    (0x3ffffe6, 26),
    (0x7ffffe0, 27),
    (0x7ffffe1, 27),
    (0x3ffffe7, 26),
    (0x7ffffe2, 27),
    (0xfffff2, 24),
    (0x1fffe4, 21),
    (0x1fffe5, 21),
    (0x3ffffe8, 26),
    (0x3ffffe9, 26),
    (0xffffffd, 28),
    (0x7ffffe3, 27),
    (0x7ffffe4, 27),
    (0x7ffffe5, 27),
    (0xfffec, 20),
    (0xfffff3, 24),
    (0xfffed, 20),
    (0x1fffe6, 21),
    (0x3fffe9, 22),
    (0x1fffe7, 21),
    (0x1fffe8, 21),
    (0x7ffff3, 23),
    (0x3fffea, 22),
    (0x3fffeb, 22),
    (0x1ffffee, 25),
    (0x1ffffef, 25),
    (0xfffff4, 24),
    (0xfffff5, 24),
    (0x3ffffea, 26),
    (0x7ffff4, 23),
    (0x3ffffeb, 26),
    (0x7ffffe6, 27),
    (0x3ffffec, 26),
    (0x3ffffed, 26),
    (0x7ffffe7, 27),
    (0x7ffffe8, 27),
    (0x7ffffe9, 27),
    (0x7ffffea, 27),
    (0x7ffffeb, 27),
    (0xffffffe, 28),
    (0x7ffffec, 27),
    (0x7ffffed, 27),
    (0x7ffffee, 27),
    (0x7ffffef, 27),
    (0x7fffff0, 27),
    (0x3ffffee, 26),
    (0x3fffffff, 30),
];

const FLAG_SYM: u8 = 1;
const FLAG_FAIL: u8 = 2;
const FLAG_MAYBE_EOS: u8 = 4;

/// Static table from RFC 7541 Appendix A.
/// Each entry: (1-based index, name, value).
const STATIC_TABLE_ENTRIES: &[(usize, &str, &str)] = &[
    (1, ":authority", ""),
    (2, ":method", "GET"),
    (3, ":method", "POST"),
    (4, ":path", "/"),
    (5, ":path", "/index.html"),
    (6, ":scheme", "http"),
    (7, ":scheme", "https"),
    (8, ":status", "200"),
    (9, ":status", "204"),
    (10, ":status", "206"),
    (11, ":status", "304"),
    (12, ":status", "400"),
    (13, ":status", "404"),
    (14, ":status", "500"),
    (15, "accept-charset", ""),
    (16, "accept-encoding", "gzip, deflate"),
    (17, "accept-language", ""),
    (18, "accept-ranges", ""),
    (19, "accept", ""),
    (20, "access-control-allow-origin", ""),
    (21, "age", ""),
    (22, "allow", ""),
    (23, "authorization", ""),
    (24, "cache-control", ""),
    (25, "content-disposition", ""),
    (26, "content-encoding", ""),
    (27, "content-language", ""),
    (28, "content-length", ""),
    (29, "content-location", ""),
    (30, "content-range", ""),
    (31, "content-type", ""),
    (32, "cookie", ""),
    (33, "date", ""),
    (34, "etag", ""),
    (35, "expect", ""),
    (36, "expires", ""),
    (37, "from", ""),
    (38, "host", ""),
    (39, "if-match", ""),
    (40, "if-modified-since", ""),
    (41, "if-none-match", ""),
    (42, "if-range", ""),
    (43, "if-unmodified-since", ""),
    (44, "last-modified", ""),
    (45, "link", ""),
    (46, "location", ""),
    (47, "max-forwards", ""),
    (48, "proxy-authenticate", ""),
    (49, "proxy-authorization", ""),
    (50, "range", ""),
    (51, "referer", ""),
    (52, "refresh", ""),
    (53, "retry-after", ""),
    (54, "server", ""),
    (55, "set-cookie", ""),
    (56, "strict-transport-security", ""),
    (57, "transfer-encoding", ""),
    (58, "user-agent", ""),
    (59, "vary", ""),
    (60, "via", ""),
    (61, "www-authenticate", ""),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set by cargo");

    generate_huffman_decode_table(&out_dir)?;
    generate_static_table_lookup(&out_dir)?;

    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}

fn generate_huffman_decode_table(out_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dest_path = Path::new(out_dir).join("huffman_decode_table.rs");

    let table = build_decode_table();

    let mut f = fs::File::create(&dest_path).map_err(|e| {
        format!(
            "failed to create huffman decode table at {}: {}",
            dest_path.display(),
            e
        )
    })?;

    writeln!(f, "/// Auto-generated Huffman decode table. Do not edit.")?;
    writeln!(f, "const FLAG_MAYBE_EOS: u8 = {};", FLAG_MAYBE_EOS)?;
    writeln!(f)?;
    writeln!(f, "static DECODE_TABLE: [DecodeEntry; {}] = [", table.len())?;
    for entry in &table {
        writeln!(
            f,
            "    DecodeEntry {{ next_state: {}, emit: {}, flags: {} }},",
            entry.next_state, entry.emit, entry.flags
        )?;
    }
    writeln!(f, "];")?;

    Ok(())
}

fn generate_static_table_lookup(out_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dest_path = Path::new(out_dir).join("static_table_lookup.rs");
    let mut f = fs::File::create(&dest_path)?;

    // Collect unique names with their first 1-based index.
    let mut unique_names: Vec<(usize, &str)> = Vec::new();
    let mut first_idx_of: HashMap<&str, usize> = HashMap::new();
    for &(idx, name, _) in STATIC_TABLE_ENTRIES {
        if let Entry::Vacant(e) = first_idx_of.entry(name) {
            e.insert(idx);
            unique_names.push((idx, name));
        }
    }

    // Group unique names by byte length.
    let mut by_length: HashMap<usize, Vec<(usize, &str)>> = HashMap::new();
    for &(idx, name) in &unique_names {
        by_length.entry(name.len()).or_default().push((idx, name));
    }

    // Collect names that have non-empty values in the static table.
    let mut names_with_values: HashMap<usize, Vec<(usize, &str)>> = HashMap::new();
    for &(idx, name, value) in STATIC_TABLE_ENTRIES {
        if !value.is_empty() {
            names_with_values
                .entry(first_idx_of[name])
                .or_default()
                .push((idx, value));
        }
    }

    // --- Generate find_in_static_table ---
    writeln!(f, "/// Auto-generated static table lookup. Do not edit.")?;
    writeln!(f, "///")?;
    writeln!(
        f,
        "/// Length-bucketed match replacing the previous `LazyLock<HashMap>` lookup."
    )?;
    writeln!(f, "#[inline]")?;
    writeln!(
        f,
        "pub fn find_in_static_table(name: &[u8], value: &[u8]) -> Option<(usize, bool)> {{"
    )?;
    writeln!(f, "    let name_idx = match name.len() {{")?;

    let mut lengths: Vec<usize> = by_length.keys().copied().collect();
    lengths.sort();

    for len in &lengths {
        let names = &by_length[len];
        if names.len() == 1 {
            let (idx, name) = names[0];
            writeln!(
                f,
                "        {len} => if name == b\"{name}\" {{ {idx} }} else {{ return None }},",
            )?;
        } else {
            // Multiple names at this length — dispatch on first byte.
            let mut by_first: HashMap<u8, Vec<(usize, &str)>> = HashMap::new();
            for &(idx, name) in names {
                by_first
                    .entry(name.as_bytes()[0])
                    .or_default()
                    .push((idx, name));
            }
            writeln!(f, "        {len} => match name[0] {{")?;
            let mut first_bytes: Vec<u8> = by_first.keys().copied().collect();
            first_bytes.sort();
            for fb in &first_bytes {
                let group = &by_first[fb];
                let ch = *fb as char;
                if group.len() == 1 {
                    let (idx, name) = group[0];
                    writeln!(
                        f,
                        "            b'{ch}' => if name == b\"{name}\" {{ {idx} }} else {{ return \
                         None }},"
                    )?;
                } else {
                    write!(f, "            b'{ch}' => ")?;
                    for (i, &(idx, name)) in group.iter().enumerate() {
                        if i == 0 {
                            write!(f, "if name == b\"{name}\" {{ {idx} }}")?;
                        } else {
                            write!(f, " else if name == b\"{name}\" {{ {idx} }}")?;
                        }
                    }
                    writeln!(f, " else {{ return None }},")?;
                }
            }
            writeln!(f, "            _ => return None,")?;
            writeln!(f, "        }},")?;
        }
    }

    writeln!(f, "        _ => return None,")?;
    writeln!(f, "    }};")?;
    writeln!(f, "    check_value(name_idx, value)")?;
    writeln!(f, "}}")?;
    writeln!(f)?;

    // --- Generate check_value ---
    writeln!(f, "#[inline]")?;
    writeln!(
        f,
        "fn check_value(name_idx: usize, value: &[u8]) -> Option<(usize, bool)> {{"
    )?;
    writeln!(f, "    match name_idx {{")?;

    let mut value_idxs: Vec<usize> = names_with_values.keys().copied().collect();
    value_idxs.sort();
    for first_idx in &value_idxs {
        let values = &names_with_values[first_idx];
        write!(f, "        {first_idx} => match value {{ ")?;
        for &(idx, value) in values {
            write!(f, "b\"{value}\" => Some(({idx}, true)), ")?;
        }
        writeln!(f, "_ => Some(({first_idx}, false)) }},")?;
    }

    writeln!(f, "        idx => Some((idx, value.is_empty())),")?;
    writeln!(f, "    }}")?;
    writeln!(f, "}}")?;

    Ok(())
}

struct DecodeEntry {
    next_state: u16,
    emit: u16,
    flags: u8,
}

struct TrieNode {
    children: [Option<usize>; 2],
    symbol: Option<u16>,
}

fn build_decode_table() -> Vec<DecodeEntry> {
    let mut nodes: Vec<TrieNode> = vec![TrieNode {
        children: [None; 2],
        symbol: None,
    }];

    for (sym, &(code, nbits)) in HUFFMAN_TABLE.iter().enumerate() {
        let mut cur = 0;
        for i in (0..nbits).rev() {
            let bit = ((code >> i) & 1) as usize;
            if nodes[cur].children[bit].is_none() {
                nodes.push(TrieNode {
                    children: [None; 2],
                    symbol: None,
                });
                let idx = nodes.len() - 1;
                nodes[cur].children[bit] = Some(idx);
            }
            cur = nodes[cur].children[bit].unwrap();
        }
        nodes[cur].symbol = Some(sym as u16);
    }

    // Compute EOS prefix nodes: trie nodes reachable from root by following
    // only 1-bits (EOS prefix path) for up to 7 bits. These are valid
    // "accept" states for Huffman padding validation (RFC 7541 §5.2).
    let mut eos_prefix_nodes: HashSet<usize> = HashSet::new();
    eos_prefix_nodes.insert(0); // root is always a valid end state
    let mut cur_eos = 0usize;
    for _ in 0..7 {
        if let Some(next) = nodes[cur_eos].children[1] {
            if nodes[next].symbol.is_some() {
                // Hit a terminal symbol — stop (but don't add it)
                break;
            }
            eos_prefix_nodes.insert(next);
            cur_eos = next;
        } else {
            break;
        }
    }

    let mut trie_to_state: HashMap<usize, u16> = HashMap::new();
    let mut state_to_trie: Vec<usize> = Vec::new();
    let mut queue: VecDeque<usize> = VecDeque::new();

    trie_to_state.insert(0, 0);
    state_to_trie.push(0);
    queue.push_back(0);

    let default_entry = DecodeEntry {
        next_state: 0,
        emit: 0,
        flags: FLAG_FAIL,
    };
    let mut table: Vec<DecodeEntry> = Vec::new();

    while let Some(trie_node) = queue.pop_front() {
        let state_idx = trie_to_state[&trie_node];
        while table.len() < (state_idx as usize + 1) * 16 {
            table.push(DecodeEntry {
                next_state: default_entry.next_state,
                emit: default_entry.emit,
                flags: default_entry.flags,
            });
        }

        for nibble in 0u8..16 {
            let mut cur = trie_node;
            let mut emitted: Option<u16> = None;
            let mut failed = false;

            for bit_pos in (0..4).rev() {
                let bit = ((nibble >> bit_pos) & 1) as usize;
                match nodes[cur].children[bit] {
                    Some(next) => {
                        cur = next;
                        if let Some(sym) = nodes[cur].symbol {
                            if sym == 256 {
                                failed = true;
                                break;
                            }
                            emitted = Some(sym);
                            cur = 0;
                        }
                    }
                    None => {
                        failed = true;
                        break;
                    }
                }
            }

            if failed {
                table[state_idx as usize * 16 + nibble as usize] = DecodeEntry {
                    next_state: 0,
                    emit: 0,
                    flags: FLAG_FAIL,
                };
                continue;
            }

            if let Entry::Vacant(e) = trie_to_state.entry(cur) {
                let new_state = state_to_trie.len() as u16;
                e.insert(new_state);
                state_to_trie.push(cur);
                queue.push_back(cur);
            }

            let next_state = trie_to_state[&cur];
            let (emit_val, mut flags) = match emitted {
                Some(sym) => (sym + 1, FLAG_SYM),
                None => (0, 0),
            };
            if eos_prefix_nodes.contains(&cur) {
                flags |= FLAG_MAYBE_EOS;
            }

            table[state_idx as usize * 16 + nibble as usize] = DecodeEntry {
                next_state,
                emit: emit_val,
                flags,
            };
        }
    }

    table
}
