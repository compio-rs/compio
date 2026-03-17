use bytes::Bytes;

use super::{FRAME_TYPE_DATA, FrameHeader, stream_id::StreamId};
use crate::error::FrameError;

const FLAG_END_STREAM: u8 = 0x1;
const FLAG_PADDED: u8 = 0x8;

/// DATA frame (type=0x0).
#[derive(Debug, Clone)]
pub struct Data {
    stream_id: StreamId,
    payload: Bytes,
    flags: u8,
    flow_controlled_len: u32,
}

impl Data {
    /// Create a new DATA frame with the given stream ID and payload.
    pub fn new(stream_id: StreamId, payload: Bytes) -> Self {
        let flow_controlled_len = payload.len() as u32;
        Data {
            stream_id,
            payload,
            flags: 0,
            flow_controlled_len,
        }
    }

    /// The stream identifier for this DATA frame.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// A reference to the frame payload data.
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    /// Consume this frame and return the payload data.
    pub fn into_payload(self) -> Bytes {
        self.payload
    }

    /// Whether the END_STREAM flag (0x1) is set.
    pub fn is_end_stream(&self) -> bool {
        self.flags & FLAG_END_STREAM != 0
    }

    /// Set the END_STREAM flag, indicating no further data will be sent on
    /// this stream.
    pub fn set_end_stream(&mut self) {
        self.flags |= FLAG_END_STREAM;
    }

    /// Whether the PADDED flag (0x8) is set.
    pub fn is_padded(&self) -> bool {
        self.flags & FLAG_PADDED != 0
    }

    /// The raw flags byte.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// The number of bytes charged against the flow control window.
    /// This is the full payload length including padding (per RFC 7540 §6.9.1).
    pub fn flow_controlled_len(&self) -> u32 {
        self.flow_controlled_len
    }

    /// Decode a DATA frame from the payload bytes (after the 9-byte header).
    pub fn decode(stream_id: StreamId, flags: u8, payload: Bytes) -> Result<Self, FrameError> {
        if stream_id.is_zero() {
            return Err(FrameError::InvalidStreamId(
                "DATA frame with stream ID 0".into(),
            ));
        }

        let flow_controlled_len = payload.len() as u32;

        let actual_payload = if flags & FLAG_PADDED != 0 {
            if payload.is_empty() {
                return Err(FrameError::InvalidPadding(
                    "padded DATA frame with no pad length".into(),
                ));
            }
            let pad_len = payload[0] as usize;
            if pad_len >= payload.len() {
                return Err(FrameError::InvalidPadding(
                    "pad length exceeds frame payload".into(),
                ));
            }
            payload.slice(1..payload.len() - pad_len)
        } else {
            payload
        };

        Ok(Data {
            stream_id,
            payload: actual_payload,
            flags,
            flow_controlled_len,
        })
    }

    /// Encode this DATA frame into bytes (header + payload).
    pub fn encode(&self, dst: &mut Vec<u8>) {
        let len = self.payload.len() as u32;
        dst.extend_from_slice(
            &FrameHeader::new(FRAME_TYPE_DATA, self.flags, self.stream_id, len).encode(),
        );
        dst.extend_from_slice(&self.payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_roundtrip() {
        let mut frame = Data::new(StreamId::new(1), Bytes::from_static(b"hello"));
        frame.set_end_stream();

        let mut buf = Vec::new();
        frame.encode(&mut buf);

        // Parse header
        assert_eq!(buf.len(), 9 + 5);
        let len = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        assert_eq!(len, 5);
        assert_eq!(buf[3], 0x0); // DATA type
        let flags = buf[4];
        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);

        let decoded =
            Data::decode(StreamId::new(sid), flags, Bytes::copy_from_slice(&buf[9..])).unwrap();
        assert_eq!(decoded.stream_id().value(), 1);
        assert!(decoded.is_end_stream());
        assert_eq!(decoded.payload().as_ref(), b"hello");
    }

    #[test]
    fn test_non_padded_flow_controlled_len() {
        let frame = Data::decode(StreamId::new(1), 0, Bytes::from_static(b"hello")).unwrap();
        assert_eq!(frame.flow_controlled_len(), 5);
        assert_eq!(frame.payload().len(), 5);
    }

    #[test]
    fn test_padded_flow_controlled_len() {
        // Build a padded DATA frame payload:
        // [pad_len=3] [data: "hi"] [padding: 0,0,0]
        let raw_payload = Bytes::from_static(&[3, b'h', b'i', 0, 0, 0]);
        let frame = Data::decode(StreamId::new(1), FLAG_PADDED, raw_payload).unwrap();

        // flow_controlled_len includes pad_len byte + data + padding = 6
        assert_eq!(frame.flow_controlled_len(), 6);
        // payload is just the data after stripping padding
        assert_eq!(frame.payload().as_ref(), b"hi");
    }
}
