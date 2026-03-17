use super::{FRAME_TYPE_RST_STREAM, FrameHeader, stream_id::StreamId};
use crate::error::{FrameError, Reason};

/// RST_STREAM frame (type=0x3).
#[derive(Debug, Clone)]
pub struct RstStream {
    stream_id: StreamId,
    reason: Reason,
}

impl RstStream {
    /// Create a new RST_STREAM frame with the given stream ID and error
    /// reason.
    pub fn new(stream_id: StreamId, reason: Reason) -> Self {
        RstStream { stream_id, reason }
    }

    /// The stream identifier being reset.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// The error code indicating why the stream is being terminated.
    pub fn reason(&self) -> Reason {
        self.reason
    }

    /// Decodes a RST_STREAM frame from the given stream ID and 4-byte payload.
    pub fn decode(stream_id: StreamId, payload: &[u8]) -> Result<Self, FrameError> {
        if stream_id.is_zero() {
            return Err(FrameError::InvalidStreamId(
                "RST_STREAM with stream ID 0".into(),
            ));
        }
        if payload.len() != 4 {
            return Err(FrameError::InvalidFrameSize(
                "RST_STREAM payload must be 4 bytes".into(),
            ));
        }

        let code = ((payload[0] as u32) << 24)
            | ((payload[1] as u32) << 16)
            | ((payload[2] as u32) << 8)
            | (payload[3] as u32);
        let reason = Reason::from_u32(code);

        Ok(RstStream { stream_id, reason })
    }

    /// Encodes this RST_STREAM frame (header + 4-byte error code) into `dst`.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        dst.extend_from_slice(
            &FrameHeader::new(FRAME_TYPE_RST_STREAM, 0, self.stream_id, 4).encode(),
        );

        let code = self.reason.to_u32();
        dst.push((code >> 24) as u8);
        dst.push((code >> 16) as u8);
        dst.push((code >> 8) as u8);
        dst.push(code as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rst_stream_roundtrip() {
        let frame = RstStream::new(StreamId::new(1), Reason::Cancel);
        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);
        let decoded = RstStream::decode(StreamId::new(sid), &buf[9..]).unwrap();
        assert_eq!(decoded.stream_id().value(), 1);
        assert_eq!(decoded.reason(), Reason::Cancel);
    }
}
