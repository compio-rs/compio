#![no_main]
use compio_h2::HpackDecoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = HpackDecoder::new(4096);
    let _ = decoder.decode(data);
});
