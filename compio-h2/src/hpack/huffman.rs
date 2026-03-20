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
    (0x14, 6),       // ' ' (32)
    (0x3f8, 10),     // '!' (33)
    (0x3f9, 10),     // '"' (34)
    (0xffa, 12),     // '#' (35)
    (0x1ff9, 13),    // '$' (36)
    (0x15, 6),       // '%' (37)
    (0xf8, 8),       // '&' (38)
    (0x7fa, 11),     // '\'' (39)
    (0x3fa, 10),     // '(' (40)
    (0x3fb, 10),     // ')' (41)
    (0xf9, 8),       // '*' (42)
    (0x7fb, 11),     // '+' (43)
    (0xfa, 8),       // ',' (44)
    (0x16, 6),       // '-' (45)
    (0x17, 6),       // '.' (46)
    (0x18, 6),       // '/' (47)
    (0x0, 5),        // '0' (48)
    (0x1, 5),        // '1' (49)
    (0x2, 5),        // '2' (50)
    (0x19, 6),       // '3' (51)
    (0x1a, 6),       // '4' (52)
    (0x1b, 6),       // '5' (53)
    (0x1c, 6),       // '6' (54)
    (0x1d, 6),       // '7' (55)
    (0x1e, 6),       // '8' (56)
    (0x1f, 6),       // '9' (57)
    (0x5c, 7),       // ':' (58)
    (0xfb, 8),       // ';' (59)
    (0x7ffc, 15),    // '<' (60)
    (0x20, 6),       // '=' (61)
    (0xffb, 12),     // '>' (62)
    (0x3fc, 10),     // '?' (63)
    (0x1ffa, 13),    // '@' (64)
    (0x21, 6),       // 'A' (65)
    (0x5d, 7),       // 'B' (66)
    (0x5e, 7),       // 'C' (67)
    (0x5f, 7),       // 'D' (68)
    (0x60, 7),       // 'E' (69)
    (0x61, 7),       // 'F' (70)
    (0x62, 7),       // 'G' (71)
    (0x63, 7),       // 'H' (72)
    (0x64, 7),       // 'I' (73)
    (0x65, 7),       // 'J' (74)
    (0x66, 7),       // 'K' (75)
    (0x67, 7),       // 'L' (76)
    (0x68, 7),       // 'M' (77)
    (0x69, 7),       // 'N' (78)
    (0x6a, 7),       // 'O' (79)
    (0x6b, 7),       // 'P' (80)
    (0x6c, 7),       // 'Q' (81)
    (0x6d, 7),       // 'R' (82)
    (0x6e, 7),       // 'S' (83)
    (0x6f, 7),       // 'T' (84)
    (0x70, 7),       // 'U' (85)
    (0x71, 7),       // 'V' (86)
    (0x72, 7),       // 'W' (87)
    (0xfc, 8),       // 'X' (88)
    (0x73, 7),       // 'Y' (89)
    (0xfd, 8),       // 'Z' (90)
    (0x1ffb, 13),    // '[' (91)
    (0x7fff0, 19),   // '\\' (92)
    (0x1ffc, 13),    // ']' (93)
    (0x3ffc, 14),    // '^' (94)
    (0x22, 6),       // '_' (95)
    (0x7ffd, 15),    // '`' (96)
    (0x3, 5),        // 'a' (97)
    (0x23, 6),       // 'b' (98)
    (0x4, 5),        // 'c' (99)
    (0x24, 6),       // 'd' (100)
    (0x5, 5),        // 'e' (101)
    (0x25, 6),       // 'f' (102)
    (0x26, 6),       // 'g' (103)
    (0x27, 6),       // 'h' (104)
    (0x6, 5),        // 'i' (105)
    (0x74, 7),       // 'j' (106)
    (0x75, 7),       // 'k' (107)
    (0x28, 6),       // 'l' (108)
    (0x29, 6),       // 'm' (109)
    (0x2a, 6),       // 'n' (110)
    (0x7, 5),        // 'o' (111)
    (0x2b, 6),       // 'p' (112)
    (0x76, 7),       // 'q' (113)
    (0x2c, 6),       // 'r' (114)
    (0x8, 5),        // 's' (115)
    (0x9, 5),        // 't' (116)
    (0x2d, 6),       // 'u' (117)
    (0x77, 7),       // 'v' (118)
    (0x78, 7),       // 'w' (119)
    (0x79, 7),       // 'x' (120)
    (0x7a, 7),       // 'y' (121)
    (0x7b, 7),       // 'z' (122)
    (0x7ffe, 15),    // '{' (123)
    (0x7fc, 11),     // '|' (124)
    (0x3ffd, 14),    // '}' (125)
    (0x1ffd, 13),    // '~' (126)
    (0xffffffc, 28), // DEL (127)
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
    (0x3fffffff, 30), // EOS (256)
];

/// Encode bytes using Huffman coding.
pub fn huffman_encode(src: &[u8]) -> Vec<u8> {
    let mut dst = Vec::new();
    let mut bits: u64 = 0;
    let mut bits_left: u8 = 0;

    for &byte in src {
        let (code, nbits) = HUFFMAN_TABLE[byte as usize];
        bits = (bits << nbits) | (code as u64);
        bits_left += nbits;

        while bits_left >= 8 {
            bits_left -= 8;
            dst.push((bits >> bits_left) as u8);
        }
    }

    // Pad with EOS prefix (all 1s)
    if bits_left > 0 {
        bits = (bits << (8 - bits_left)) | ((1u64 << (8 - bits_left)) - 1);
        dst.push(bits as u8);
    }

    dst
}

/// State machine entry for 4-bit nibble-based Huffman decoding.
/// Each entry describes what happens when processing a nibble in a given state.
#[derive(Clone, Copy)]
struct DecodeEntry {
    /// Next state index (into the decode table).
    next_state: u16,
    /// If non-zero, emit (sym - 1) as a decoded byte. 0 means no emit.
    emit: u16,
    /// Flags: bit 0 = terminal (a symbol was fully decoded).
    flags: u8,
}

const FLAG_SYM: u8 = 1; // A symbol was emitted
const FLAG_FAIL: u8 = 2; // Invalid / EOS encountered

// Decode table generated at build time by build.rs.
// Table layout: for state S and nibble N, entry is at [S * 16 + N].
include!(concat!(env!("OUT_DIR"), "/huffman_decode_table.rs"));

/// Decode Huffman-encoded bytes using a nibble-based state machine.
pub fn huffman_decode(src: &[u8]) -> Result<Vec<u8>, String> {
    let table = &DECODE_TABLE;
    let mut dst = Vec::new();
    let mut state: u16 = 0;
    // State 0 (root) is always a valid accept state
    let mut maybe_eos = true;

    for &byte in src {
        // Process high nibble
        let hi = (byte >> 4) as usize;
        let entry = &table[state as usize * 16 + hi];
        if entry.flags & FLAG_FAIL != 0 {
            return Err("invalid Huffman encoding".into());
        }
        if entry.flags & FLAG_SYM != 0 {
            dst.push((entry.emit - 1) as u8);
        }
        state = entry.next_state;

        // Process low nibble
        let lo = (byte & 0x0f) as usize;
        let entry = &table[state as usize * 16 + lo];
        if entry.flags & FLAG_FAIL != 0 {
            return Err("invalid Huffman encoding".into());
        }
        if entry.flags & FLAG_SYM != 0 {
            dst.push((entry.emit - 1) as u8);
        }
        state = entry.next_state;
        maybe_eos = entry.flags & FLAG_MAYBE_EOS != 0;
    }

    // Validate padding per RFC 7541 §5.2:
    // After decoding, the state machine must be in an accepting state:
    // either back at the root (state 0) or at a node reachable from root
    // by following only 1-bits (EOS prefix) for at most 7 bits.
    if !src.is_empty() && state != 0 && !maybe_eos {
        return Err("invalid Huffman encoding: invalid padding".into());
    }

    Ok(dst)
}

/// Calculate the encoded length of a byte string under Huffman coding.
pub fn huffman_encoded_len(src: &[u8]) -> usize {
    let mut bits: usize = 0;
    for &byte in src {
        bits += HUFFMAN_TABLE[byte as usize].1 as usize;
    }
    bits.div_ceil(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_roundtrip() {
        let inputs = [
            b"www.example.com".as_ref(),
            b"no-cache",
            b"custom-key",
            b"custom-value",
            b"",
            b"GET",
            b"/",
            b"https",
        ];
        for input in &inputs {
            let encoded = huffman_encode(input);
            let decoded = huffman_decode(&encoded).unwrap();
            assert_eq!(
                &decoded,
                input,
                "roundtrip failed for {:?}",
                std::str::from_utf8(input)
            );
        }
    }

    #[test]
    fn test_huffman_encoding_shorter() {
        // Huffman encoding should generally be shorter for ASCII text
        let input = b"www.example.com";
        let encoded = huffman_encode(input);
        assert!(encoded.len() <= input.len());
    }

    #[test]
    fn test_huffman_invalid_padding() {
        // Encode "a" (Huffman code: 0x03, 5 bits = 00011)
        // Properly padded: 00011|111 = 0x1f
        // Invalid padding: 00011|000 = 0x18 (padding bits are 0, not 1)
        let invalid = vec![0x18u8];
        let result = huffman_decode(&invalid);
        assert!(result.is_err(), "should reject invalid padding");
    }

    #[test]
    fn test_huffman_excessive_padding() {
        // More than 7 bits of padding (>7 trailing 1-bits that don't decode
        // to a symbol) should be rejected per RFC 7541 §5.2.
        // "a" = 00011 (5 bits), then 11 bits of 1-padding = 00011|111 11111111
        // That's 0x1f 0xff — the second byte is all-1s EOS padding but
        // exceeds the 7-bit limit.
        let invalid = vec![0x1fu8, 0xffu8];
        let result = huffman_decode(&invalid);
        assert!(result.is_err(), "should reject >7 bits of padding");
    }

    #[test]
    fn test_huffman_valid_padding() {
        // "a" = 00011, padded with 111 -> 00011111 = 0x1f
        let valid = vec![0x1fu8];
        let result = huffman_decode(&valid);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"a");
    }

    #[test]
    fn test_huffman_encoded_len() {
        let input = b"www.example.com";
        let encoded = huffman_encode(input);
        assert_eq!(huffman_encoded_len(input), encoded.len());
    }

    // --- FLAG_MAYBE_EOS edge-case tests ---

    /// Verify that padding of 1–7 bits (all ones) is accepted.
    /// We craft strings whose Huffman encoding ends with various amounts of
    /// padding and ensure they all decode successfully.
    #[test]
    fn test_valid_padding_all_bit_lengths() {
        // For each test string, encode → decode should roundtrip.
        // The Huffman codes of these characters produce varying padding widths.
        let test_strings: &[&[u8]] = &[
            b"0",      // '0' = 5 bits → 3 bits padding
            b"0a",     // 5+5 = 10 bits → 6 bits padding
            b"www",    // 7+7+7 = 21 bits → 3 bits padding
            b"ab",     // 5+6 = 11 bits → 5 bits padding
            b"aaa",    // 5+5+5 = 15 bits → 1 bit padding
            b"aaaa",   // 5+5+5+5 = 20 bits → 4 bits padding
            b"aaaaa",  // 5*5 = 25 bits → 7 bits padding
            b"aaaaaa", // 5*6 = 30 bits → 2 bits padding
        ];
        for input in test_strings {
            let encoded = huffman_encode(input);
            let decoded = huffman_decode(&encoded).unwrap_or_else(|e| {
                panic!(
                    "valid padding rejected for {:?}: {e}",
                    std::str::from_utf8(input)
                )
            });
            assert_eq!(&decoded, input);
        }
    }

    /// Verify that padding bits that are NOT all ones are rejected.
    #[test]
    fn test_invalid_padding_bits_rejected() {
        // "a" = 00011 (5 bits), valid padding = 111 → 00011|111 = 0x1f
        // Try each invalid padding variant (at least one 0 in padding)
        // Each byte is "a" (5-bit symbol 00011) + 3-bit padding (not all ones)
        let invalid_paddings: &[u8] = &[
            0b0001_1000, // 0x18
            0b0001_1001, // 0x19
            0b0001_1010, // 0x1a
            0b0001_1011, // 0x1b
            0b0001_1100, // 0x1c
            0b0001_1101, // 0x1d
            0b0001_1110, // 0x1e
        ];
        for &byte in invalid_paddings {
            let result = huffman_decode(&[byte]);
            assert!(
                result.is_err(),
                "should reject byte 0x{byte:02x} (non-EOS padding)"
            );
        }
    }

    /// 32 bits of all-ones (no preceding symbol) exceeds the 7-bit EOS
    /// padding limit and must be rejected.
    #[test]
    fn test_eos_in_stream_rejected() {
        let all_ones = vec![0xffu8; 4];
        let result = huffman_decode(&all_ones);
        assert!(result.is_err(), "should reject 32 bits of EOS padding");
    }

    /// Single-byte all-ones (0xff) should be rejected — it's 8 bits of EOS
    /// padding which exceeds the 7-bit limit.
    #[test]
    fn test_all_ones_byte_rejected() {
        let result = huffman_decode(&[0xff]);
        assert!(
            result.is_err(),
            "should reject 0xff (8 bits of EOS padding)"
        );
    }

    /// Verify that the decode table has FLAG_MAYBE_EOS set on root state
    /// transitions that follow EOS prefix paths. State 0 with nibble 0xf
    /// (all ones) should have FLAG_MAYBE_EOS set, since 4 bits of all-ones
    /// is a prefix of EOS.
    /// Every single byte value must roundtrip through Huffman encode/decode.
    #[test]
    fn test_huffman_roundtrip_all_bytes() {
        for byte in 0u8..=255 {
            let input = [byte];
            let encoded = huffman_encode(&input);
            let decoded = huffman_decode(&encoded).unwrap_or_else(|e| {
                panic!("decode failed for byte {byte} (0x{byte:02x}): {e}");
            });
            assert_eq!(
                decoded, input,
                "roundtrip mismatch for byte {byte} (0x{byte:02x}): encoded={encoded:02x?}, \
                 decoded={decoded:?}"
            );
        }
    }

    /// Roundtrip the exact fuzz crash input that exposed the HPACK bug.
    #[test]
    fn test_huffman_roundtrip_fuzz_crash() {
        // Byte 165 (0xa5) in combination with other bytes triggers the bug.
        let input = [165u8];
        let encoded = huffman_encode(&input);
        let decoded = huffman_decode(&encoded).unwrap();
        assert_eq!(decoded, input, "byte 165 roundtrip failed");
    }

    #[test]
    fn test_decode_table_maybe_eos_on_root() {
        let table = &DECODE_TABLE;
        // From state 0, nibble 0xf (1111) follows the EOS prefix path
        let entry = &table[0xf];
        assert!(
            entry.flags & FLAG_MAYBE_EOS != 0,
            "state 0 + nibble 0xf should have FLAG_MAYBE_EOS"
        );
    }
}
