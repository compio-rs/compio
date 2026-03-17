use bytes::Bytes;

use super::{FRAME_TYPE_GOAWAY, FrameHeader, stream_id::StreamId};
use crate::error::{FrameError, Reason};

/// GOAWAY frame (type=0x7).
#[derive(Debug, Clone)]
pub struct GoAway {
    last_stream_id: StreamId,
    error_code: Reason,
    debug_data: Bytes,
}

impl GoAway {
    /// Create a new GOAWAY frame with the given last stream ID and error code.
    pub fn new(last_stream_id: StreamId, error_code: Reason) -> Self {
        GoAway {
            last_stream_id,
            error_code,
            debug_data: Bytes::new(),
        }
    }

    /// Create a new GOAWAY frame with additional opaque debug data.
    pub fn with_debug_data(
        last_stream_id: StreamId,
        error_code: Reason,
        debug_data: Bytes,
    ) -> Self {
        GoAway {
            last_stream_id,
            error_code,
            debug_data,
        }
    }

    /// The highest-numbered stream ID the sender may have processed.
    pub fn last_stream_id(&self) -> StreamId {
        self.last_stream_id
    }

    /// The error code indicating the reason for closing the connection.
    pub fn error_code(&self) -> Reason {
        self.error_code
    }

    /// The optional opaque debug data.
    pub fn debug_data(&self) -> &Bytes {
        &self.debug_data
    }

    /// Decode a GOAWAY frame. Per RFC 7540 Section 6.8, stream_id MUST be 0.
    pub fn decode(
        stream_id: super::stream_id::StreamId,
        payload: Bytes,
    ) -> Result<Self, FrameError> {
        if !stream_id.is_zero() {
            return Err(FrameError::ProtocolError(
                "GOAWAY frame with non-zero stream ID (PROTOCOL_ERROR)".into(),
            ));
        }
        if payload.len() < 8 {
            return Err(FrameError::InvalidFrameSize(
                "GOAWAY payload too short".into(),
            ));
        }

        let last_stream_id = StreamId::new(
            ((payload[0] as u32) << 24)
                | ((payload[1] as u32) << 16)
                | ((payload[2] as u32) << 8)
                | (payload[3] as u32),
        );

        let error_code_raw = ((payload[4] as u32) << 24)
            | ((payload[5] as u32) << 16)
            | ((payload[6] as u32) << 8)
            | (payload[7] as u32);
        let error_code = Reason::from_u32(error_code_raw);

        let debug_data = if payload.len() > 8 {
            payload.slice(8..)
        } else {
            Bytes::new()
        };

        Ok(GoAway {
            last_stream_id,
            error_code,
            debug_data,
        })
    }

    /// Encodes this GOAWAY frame (header + payload) into `dst`.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        let payload_len = (8 + self.debug_data.len()) as u32;
        dst.extend_from_slice(
            &FrameHeader::new(FRAME_TYPE_GOAWAY, 0, StreamId::ZERO, payload_len).encode(),
        );

        let lsid = self.last_stream_id.value();
        dst.push((lsid >> 24) as u8);
        dst.push((lsid >> 16) as u8);
        dst.push((lsid >> 8) as u8);
        dst.push(lsid as u8);

        let ec = self.error_code.to_u32();
        dst.push((ec >> 24) as u8);
        dst.push((ec >> 16) as u8);
        dst.push((ec >> 8) as u8);
        dst.push(ec as u8);

        dst.extend_from_slice(&self.debug_data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goaway_roundtrip() {
        let frame = GoAway::with_debug_data(
            StreamId::new(5),
            Reason::NoError,
            Bytes::from_static(b"bye"),
        );
        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let decoded = GoAway::decode(StreamId::ZERO, Bytes::copy_from_slice(&buf[9..])).unwrap();
        assert_eq!(decoded.last_stream_id().value(), 5);
        assert_eq!(decoded.error_code(), Reason::NoError);
        assert_eq!(decoded.debug_data().as_ref(), b"bye");
    }

    #[test]
    fn test_goaway_non_zero_stream_id() {
        let payload = Bytes::from(vec![0u8; 8]);
        let result = GoAway::decode(StreamId::new(1), payload);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("PROTOCOL_ERROR"));
    }

    #[test]
    fn test_goaway_unknown_error_code_preserved() {
        let mut payload = vec![0u8; 8]; // last_stream_id = 0
        // Set error code to 0xFF (unknown)
        payload[4] = 0;
        payload[5] = 0;
        payload[6] = 0;
        payload[7] = 0xFF;
        let frame = GoAway::decode(StreamId::ZERO, Bytes::from(payload)).unwrap();
        assert_eq!(frame.error_code(), Reason::Unknown(0xFF));
        assert_eq!(frame.error_code().to_u32(), 0xFF);
    }
}
