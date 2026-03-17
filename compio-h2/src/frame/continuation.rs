use bytes::Bytes;

use super::{FRAME_TYPE_CONTINUATION, FrameHeader, stream_id::StreamId};
use crate::error::FrameError;

const FLAG_END_HEADERS: u8 = 0x4;

/// CONTINUATION frame (type=0x9).
#[derive(Debug, Clone)]
pub struct Continuation {
    stream_id: StreamId,
    header_block: Bytes,
    flags: u8,
}

impl Continuation {
    /// Create a new CONTINUATION frame (END_HEADERS not set by default).
    pub fn new(stream_id: StreamId, header_block: Bytes) -> Self {
        Continuation {
            stream_id,
            header_block,
            flags: 0,
        }
    }

    /// The stream identifier for this CONTINUATION frame.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// A reference to the encoded header block fragment.
    pub fn header_block(&self) -> &Bytes {
        &self.header_block
    }

    /// Consume this frame and return the header block fragment.
    pub fn into_header_block(self) -> Bytes {
        self.header_block
    }

    /// Whether the END_HEADERS flag (0x4) is set.
    pub fn is_end_headers(&self) -> bool {
        self.flags & FLAG_END_HEADERS != 0
    }

    /// Set the END_HEADERS flag, indicating the header block is complete.
    pub fn set_end_headers(&mut self) {
        self.flags |= FLAG_END_HEADERS;
    }

    /// The raw flags byte.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// Decodes a CONTINUATION frame from the given stream ID, flags, and
    /// payload.
    pub fn decode(stream_id: StreamId, flags: u8, payload: Bytes) -> Result<Self, FrameError> {
        if stream_id.is_zero() {
            return Err(FrameError::InvalidStreamId(
                "CONTINUATION with stream ID 0".into(),
            ));
        }

        Ok(Continuation {
            stream_id,
            header_block: payload,
            flags,
        })
    }

    /// Encodes this CONTINUATION frame (header + payload) into `dst`.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        let len = self.header_block.len() as u32;
        dst.extend_from_slice(
            &FrameHeader::new(FRAME_TYPE_CONTINUATION, self.flags, self.stream_id, len).encode(),
        );
        dst.extend_from_slice(&self.header_block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_continuation_roundtrip() {
        let mut frame = Continuation::new(StreamId::new(1), Bytes::from_static(b"\x82\x86"));
        frame.set_end_headers();

        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let flags = buf[4];
        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);
        let decoded =
            Continuation::decode(StreamId::new(sid), flags, Bytes::copy_from_slice(&buf[9..]))
                .unwrap();
        assert_eq!(decoded.stream_id().value(), 1);
        assert!(decoded.is_end_headers());
        assert_eq!(decoded.header_block().as_ref(), b"\x82\x86");
    }
}
