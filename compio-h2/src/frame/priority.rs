use super::{FRAME_TYPE_PRIORITY, FrameHeader, stream_id::StreamId};
use crate::error::FrameError;

/// PRIORITY frame (type=0x2).
#[derive(Debug, Clone)]
pub struct Priority {
    stream_id: StreamId,
    exclusive: bool,
    dependency: StreamId,
    weight: u8,
}

impl Priority {
    /// Create a new PRIORITY frame with the given stream dependency and
    /// weight.
    pub fn new(stream_id: StreamId, exclusive: bool, dependency: StreamId, weight: u8) -> Self {
        Priority {
            stream_id,
            exclusive,
            dependency,
            weight,
        }
    }

    /// The stream identifier for this PRIORITY frame.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Whether the exclusive dependency flag is set.
    pub fn exclusive(&self) -> bool {
        self.exclusive
    }

    /// The stream ID that this stream depends on.
    pub fn dependency(&self) -> StreamId {
        self.dependency
    }

    /// The priority weight (0-255, actual weight is value + 1).
    pub fn weight(&self) -> u8 {
        self.weight
    }

    /// Decodes a PRIORITY frame from the given stream ID and 5-byte payload.
    pub fn decode(stream_id: StreamId, payload: &[u8]) -> Result<Self, FrameError> {
        if stream_id.is_zero() {
            return Err(FrameError::InvalidStreamId(
                "PRIORITY with stream ID 0".into(),
            ));
        }
        if payload.len() != 5 {
            return Err(FrameError::InvalidFrameSize(
                "PRIORITY payload must be 5 bytes".into(),
            ));
        }

        let dep_raw = ((payload[0] as u32) << 24)
            | ((payload[1] as u32) << 16)
            | ((payload[2] as u32) << 8)
            | (payload[3] as u32);
        let exclusive = dep_raw & 0x8000_0000 != 0;
        let dependency = StreamId::new(dep_raw & 0x7FFF_FFFF);
        let weight = payload[4];

        if dependency == stream_id {
            return Err(FrameError::ProtocolError(format!(
                "PRIORITY frame with self-dependency on stream {}",
                stream_id.value()
            )));
        }

        Ok(Priority {
            stream_id,
            exclusive,
            dependency,
            weight,
        })
    }

    /// Encodes this PRIORITY frame (header + 5-byte payload) into `dst`.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        dst.extend_from_slice(
            &FrameHeader::new(FRAME_TYPE_PRIORITY, 0, self.stream_id, 5).encode(),
        );

        let dep = self.dependency.value() | if self.exclusive { 0x8000_0000 } else { 0 };
        dst.push((dep >> 24) as u8);
        dst.push((dep >> 16) as u8);
        dst.push((dep >> 8) as u8);
        dst.push(dep as u8);
        dst.push(self.weight);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_self_dependency_rejected() {
        // dependency == stream_id → ProtocolError
        let mut payload = [0u8; 5];
        let dep: u32 = 7; // same as stream_id
        payload[0] = (dep >> 24) as u8;
        payload[1] = (dep >> 16) as u8;
        payload[2] = (dep >> 8) as u8;
        payload[3] = dep as u8;
        payload[4] = 16; // weight

        let result = Priority::decode(StreamId::new(7), &payload);
        assert!(result.is_err());
        match result.unwrap_err() {
            FrameError::ProtocolError(msg) => assert!(msg.contains("self-dependency")),
            other => panic!("expected ProtocolError, got {:?}", other),
        }
    }

    #[test]
    fn test_priority_different_dependency_ok() {
        let mut payload = [0u8; 5];
        let dep: u32 = 1;
        payload[0] = (dep >> 24) as u8;
        payload[1] = (dep >> 16) as u8;
        payload[2] = (dep >> 8) as u8;
        payload[3] = dep as u8;
        payload[4] = 16;

        let result = Priority::decode(StreamId::new(7), &payload);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().dependency().value(), 1);
    }

    #[test]
    fn test_priority_roundtrip() {
        let frame = Priority::new(StreamId::new(3), true, StreamId::new(1), 200);
        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);
        let decoded = Priority::decode(StreamId::new(sid), &buf[9..]).unwrap();
        assert_eq!(decoded.stream_id().value(), 3);
        assert!(decoded.exclusive());
        assert_eq!(decoded.dependency().value(), 1);
        assert_eq!(decoded.weight(), 200);
    }
}
