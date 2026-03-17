use super::{FRAME_TYPE_WINDOW_UPDATE, FrameHeader, stream_id::StreamId};
use crate::error::FrameError;

/// WINDOW_UPDATE frame (type=0x8).
#[derive(Debug, Clone)]
pub struct WindowUpdate {
    stream_id: StreamId,
    size_increment: u32,
}

impl WindowUpdate {
    /// Mask for 31-bit size increment.
    const MASK: u32 = 0x7FFF_FFFF;

    /// Create a new WINDOW_UPDATE frame with the given stream ID and size
    /// increment.
    pub fn new(stream_id: StreamId, size_increment: u32) -> Self {
        WindowUpdate {
            stream_id,
            size_increment: size_increment & Self::MASK,
        }
    }

    /// The stream ID, or zero for a connection-level window update.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// The flow-control window size increment (31-bit unsigned).
    pub fn size_increment(&self) -> u32 {
        self.size_increment
    }

    /// Decodes a WINDOW_UPDATE frame from the given stream ID and 4-byte
    /// payload.
    pub fn decode(stream_id: StreamId, payload: &[u8]) -> Result<Self, FrameError> {
        if payload.len() != 4 {
            return Err(FrameError::InvalidFrameSize(
                "WINDOW_UPDATE payload must be 4 bytes".into(),
            ));
        }

        let raw = ((payload[0] as u32) << 24)
            | ((payload[1] as u32) << 16)
            | ((payload[2] as u32) << 8)
            | (payload[3] as u32);
        let size_increment = raw & Self::MASK;

        if size_increment == 0 {
            return Err(FrameError::ProtocolError(
                "WINDOW_UPDATE with zero increment".into(),
            ));
        }

        Ok(WindowUpdate {
            stream_id,
            size_increment,
        })
    }

    /// Encodes this WINDOW_UPDATE frame (header + 4-byte payload) into `dst`.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        dst.extend_from_slice(
            &FrameHeader::new(FRAME_TYPE_WINDOW_UPDATE, 0, self.stream_id, 4).encode(),
        );
        // Payload
        let inc = self.size_increment & Self::MASK;
        dst.push((inc >> 24) as u8);
        dst.push((inc >> 16) as u8);
        dst.push((inc >> 8) as u8);
        dst.push(inc as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_update_zero_increment_rejected() {
        let payload = [0u8, 0, 0, 0]; // increment = 0
        let result = WindowUpdate::decode(StreamId::new(1), &payload);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            format!("{}", err).contains("zero"),
            "expected zero increment error, got: {}",
            err
        );
    }

    #[test]
    fn test_window_update_roundtrip() {
        let frame = WindowUpdate::new(StreamId::new(1), 65535);
        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);
        let decoded = WindowUpdate::decode(StreamId::new(sid), &buf[9..]).unwrap();
        assert_eq!(decoded.stream_id().value(), 1);
        assert_eq!(decoded.size_increment(), 65535);
    }
}
