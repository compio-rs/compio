use super::{FRAME_TYPE_PING, FrameHeader};
use crate::error::FrameError;

const FLAG_ACK: u8 = 0x1;

/// PING frame (type=0x6).
#[derive(Debug, Clone)]
pub struct Ping {
    opaque_data: [u8; 8],
    flags: u8,
}

impl Ping {
    /// Create a new PING frame with the given 8-byte opaque data.
    pub fn new(opaque_data: [u8; 8]) -> Self {
        Ping {
            opaque_data,
            flags: 0,
        }
    }

    /// Create a PING acknowledgement (ACK flag set) echoing back the opaque
    /// data.
    pub fn pong(opaque_data: [u8; 8]) -> Self {
        Ping {
            opaque_data,
            flags: FLAG_ACK,
        }
    }

    /// The 8-byte opaque data payload.
    pub fn opaque_data(&self) -> &[u8; 8] {
        &self.opaque_data
    }

    /// Whether this is a PING acknowledgement (ACK flag set).
    pub fn is_ack(&self) -> bool {
        self.flags & FLAG_ACK != 0
    }

    /// The raw flags byte.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// Decode a PING frame. Per RFC 7540 Section 6.7, stream_id MUST be 0.
    pub fn decode(
        stream_id: super::stream_id::StreamId,
        flags: u8,
        payload: &[u8],
    ) -> Result<Self, FrameError> {
        if !stream_id.is_zero() {
            return Err(FrameError::ProtocolError(
                "PING frame with non-zero stream ID (PROTOCOL_ERROR)".into(),
            ));
        }
        if payload.len() != 8 {
            return Err(FrameError::InvalidFrameSize(
                "PING payload must be 8 bytes".into(),
            ));
        }
        let mut opaque_data = [0u8; 8];
        opaque_data.copy_from_slice(payload);
        Ok(Ping { opaque_data, flags })
    }

    /// Encodes this PING frame (header + 8-byte payload) into `dst`.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        dst.extend_from_slice(
            &FrameHeader::new(
                FRAME_TYPE_PING,
                self.flags,
                super::stream_id::StreamId::ZERO,
                8,
            )
            .encode(),
        );
        dst.extend_from_slice(&self.opaque_data);
    }
}

#[cfg(test)]
mod tests {
    use super::{super::stream_id::StreamId, *};

    #[test]
    fn test_ping_roundtrip() {
        let frame = Ping::new([1, 2, 3, 4, 5, 6, 7, 8]);
        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let flags = buf[4];
        let decoded = Ping::decode(StreamId::ZERO, flags, &buf[9..]).unwrap();
        assert_eq!(decoded.opaque_data(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(!decoded.is_ack());
    }

    #[test]
    fn test_pong() {
        let frame = Ping::pong([1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(frame.is_ack());
    }

    #[test]
    fn test_ping_non_zero_stream_id() {
        let payload = [0u8; 8];
        let result = Ping::decode(StreamId::new(1), 0, &payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("PROTOCOL_ERROR"));
    }
}
