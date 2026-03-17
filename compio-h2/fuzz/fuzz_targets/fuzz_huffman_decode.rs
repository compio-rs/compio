#![no_main]
use compio_h2::hpack::huffman::huffman_decode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = huffman_decode(data);
});
