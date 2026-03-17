#![no_main]
use bytes::Bytes;
use compio_h2::{Frame, FrameHeader};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 9 {
        return;
    }
    let header = FrameHeader::decode(data[..9].try_into().unwrap());
    let payload_len = header.length as usize;
    if data.len() < 9 + payload_len {
        return;
    }
    let payload = Bytes::copy_from_slice(&data[9..9 + payload_len]);
    let _ = Frame::decode(header, payload);
});
