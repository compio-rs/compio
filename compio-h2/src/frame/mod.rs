/// CONTINUATION frame type module.
pub mod continuation;
/// DATA frame type module.
pub mod data;
/// GOAWAY frame type module.
pub mod goaway;
/// HEADERS frame type module.
pub mod headers;
/// PING frame type module.
pub mod ping;
/// PRIORITY frame type module.
pub mod priority;
/// RST_STREAM frame type module.
pub mod rst_stream;
/// SETTINGS frame type module.
pub mod settings;
/// Stream identifier type module.
pub mod stream_id;
/// WINDOW_UPDATE frame type module.
pub mod window_update;

use bytes::Bytes;

pub use self::{
    continuation::Continuation, data::Data, goaway::GoAway, headers::Headers, ping::Ping,
    priority::Priority, rst_stream::RstStream, settings::Settings, stream_id::StreamId,
    window_update::WindowUpdate,
};
use crate::error::FrameError;

/// Frame type identifier for DATA frames (RFC 9113 Section 6.1).
pub const FRAME_TYPE_DATA: u8 = 0x0;
/// Frame type identifier for HEADERS frames (RFC 9113 Section 6.2).
pub const FRAME_TYPE_HEADERS: u8 = 0x1;
/// Frame type identifier for PRIORITY frames (RFC 9113 Section 6.3).
pub const FRAME_TYPE_PRIORITY: u8 = 0x2;
/// Frame type identifier for RST_STREAM frames (RFC 9113 Section 6.4).
pub const FRAME_TYPE_RST_STREAM: u8 = 0x3;
/// Frame type identifier for SETTINGS frames (RFC 9113 Section 6.5).
pub const FRAME_TYPE_SETTINGS: u8 = 0x4;
/// Frame type identifier for PUSH_PROMISE frames (RFC 9113 Section 6.6).
pub const FRAME_TYPE_PUSH_PROMISE: u8 = 0x5;
/// Frame type identifier for PING frames (RFC 9113 Section 6.7).
pub const FRAME_TYPE_PING: u8 = 0x6;
/// Frame type identifier for GOAWAY frames (RFC 9113 Section 6.8).
pub const FRAME_TYPE_GOAWAY: u8 = 0x7;
/// Frame type identifier for WINDOW_UPDATE frames (RFC 9113 Section 6.9).
pub const FRAME_TYPE_WINDOW_UPDATE: u8 = 0x8;
/// Frame type identifier for CONTINUATION frames (RFC 9113 Section 6.10).
pub const FRAME_TYPE_CONTINUATION: u8 = 0x9;

/// Size of an HTTP/2 frame header in bytes.
pub const FRAME_HEADER_SIZE: usize = 9;

/// Maximum allowed frame payload length (2^14 default, 2^24-1 max).
pub const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;
/// Maximum allowable value for SETTINGS_MAX_FRAME_SIZE (2^24-1, per RFC 9113
/// Section 4.2).
pub const MAX_FRAME_SIZE_UPPER: u32 = 16_777_215;

/// The HTTP/2 connection preface.
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Parsed frame header (9 bytes).
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    /// Length of the frame payload in bytes (24-bit).
    pub length: u32,
    /// Frame type identifier (e.g., DATA=0x0, HEADERS=0x1).
    pub frame_type: u8,
    /// Frame-type-specific flags.
    pub flags: u8,
    /// Stream identifier this frame belongs to, or zero for connection frames.
    pub stream_id: StreamId,
}

impl FrameHeader {
    /// Create a new frame header with the given type, flags, stream ID, and
    /// payload length.
    pub fn new(frame_type: u8, flags: u8, stream_id: StreamId, length: u32) -> Self {
        FrameHeader {
            length,
            frame_type,
            flags,
            stream_id,
        }
    }

    /// Decode a frame header from 9 bytes.
    pub fn decode(src: &[u8; 9]) -> Self {
        let length = ((src[0] as u32) << 16) | ((src[1] as u32) << 8) | (src[2] as u32);
        let frame_type = src[3];
        let flags = src[4];
        let stream_id = StreamId::new(
            ((src[5] as u32) << 24)
                | ((src[6] as u32) << 16)
                | ((src[7] as u32) << 8)
                | (src[8] as u32),
        );
        FrameHeader {
            length,
            frame_type,
            flags,
            stream_id,
        }
    }

    /// Encode a frame header into 9 bytes.
    pub fn encode(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = (self.length >> 16) as u8;
        buf[1] = (self.length >> 8) as u8;
        buf[2] = self.length as u8;
        buf[3] = self.frame_type;
        buf[4] = self.flags;
        let sid = self.stream_id.value();
        buf[5] = (sid >> 24) as u8;
        buf[6] = (sid >> 16) as u8;
        buf[7] = (sid >> 8) as u8;
        buf[8] = sid as u8;
        buf
    }
}

/// A parsed HTTP/2 frame.
#[derive(Debug, Clone)]
pub enum Frame {
    /// A DATA frame carrying request or response body data (RFC 9113 Section
    /// 6.1).
    Data(Data),
    /// A HEADERS frame carrying header block fragments (RFC 9113 Section 6.2).
    Headers(Headers),
    /// A PRIORITY frame specifying stream prioritization (RFC 9113 Section
    /// 6.3).
    Priority(Priority),
    /// A RST_STREAM frame for abnormal stream termination (RFC 9113 Section
    /// 6.4).
    RstStream(RstStream),
    /// A SETTINGS frame for connection configuration (RFC 9113 Section 6.5).
    Settings(Settings),
    /// A PING frame for connection liveness and RTT measurement (RFC 9113
    /// Section 6.7).
    Ping(Ping),
    /// A GOAWAY frame for graceful connection shutdown (RFC 9113 Section 6.8).
    GoAway(GoAway),
    /// A WINDOW_UPDATE frame for flow control (RFC 9113 Section 6.9).
    WindowUpdate(WindowUpdate),
    /// A CONTINUATION frame carrying additional header block fragments (RFC
    /// 9113 Section 6.10).
    Continuation(Continuation),
}

impl Frame {
    /// Parse a frame from a header and payload.
    ///
    /// `Ok(None)` is returned for unknown frame types per RFC 7540 §4.1:
    /// "Implementations MUST ignore and discard any frame that has a type that
    /// is unknown."
    pub fn decode(header: FrameHeader, payload: Bytes) -> Result<Option<Self>, FrameError> {
        // RFC 7540 §6.x: Validate stream_id constraints per frame type.
        // Stream-bearing frames MUST have non-zero stream_id;
        // connection frames MUST have stream_id=0.
        match header.frame_type {
            FRAME_TYPE_DATA
            | FRAME_TYPE_HEADERS
            | FRAME_TYPE_PRIORITY
            | FRAME_TYPE_RST_STREAM
            | FRAME_TYPE_PUSH_PROMISE
            | FRAME_TYPE_CONTINUATION => {
                if header.stream_id.is_zero() {
                    return Err(FrameError::ProtocolError(format!(
                        "frame type 0x{:x} requires non-zero stream ID",
                        header.frame_type,
                    )));
                }
            }
            FRAME_TYPE_SETTINGS | FRAME_TYPE_PING | FRAME_TYPE_GOAWAY => {
                if !header.stream_id.is_zero() {
                    return Err(FrameError::ProtocolError(format!(
                        "frame type 0x{:x} requires stream ID 0, got {}",
                        header.frame_type,
                        header.stream_id.value(),
                    )));
                }
            }
            // WINDOW_UPDATE: valid on both stream 0 and non-zero streams
            // Unknown frame types: no stream_id constraint (RFC 7540 §4.1)
            _ => {}
        }

        match header.frame_type {
            FRAME_TYPE_DATA => {
                Data::decode(header.stream_id, header.flags, payload).map(|f| Some(Frame::Data(f)))
            }
            FRAME_TYPE_HEADERS => Headers::decode(header.stream_id, header.flags, payload)
                .map(|f| Some(Frame::Headers(f))),
            FRAME_TYPE_PRIORITY => {
                Priority::decode(header.stream_id, &payload).map(|f| Some(Frame::Priority(f)))
            }
            FRAME_TYPE_RST_STREAM => {
                RstStream::decode(header.stream_id, &payload).map(|f| Some(Frame::RstStream(f)))
            }
            FRAME_TYPE_SETTINGS => Settings::decode(header.stream_id, header.flags, &payload)
                .map(|f| Some(Frame::Settings(f))),
            FRAME_TYPE_PING => {
                Ping::decode(header.stream_id, header.flags, &payload).map(|f| Some(Frame::Ping(f)))
            }
            FRAME_TYPE_GOAWAY => {
                GoAway::decode(header.stream_id, payload).map(|f| Some(Frame::GoAway(f)))
            }
            FRAME_TYPE_WINDOW_UPDATE => WindowUpdate::decode(header.stream_id, &payload)
                .map(|f| Some(Frame::WindowUpdate(f))),
            FRAME_TYPE_CONTINUATION => {
                Continuation::decode(header.stream_id, header.flags, payload)
                    .map(|f| Some(Frame::Continuation(f)))
            }
            FRAME_TYPE_PUSH_PROMISE => {
                // PUSH_PROMISE is not supported; receiving one is a connection
                // error (RFC 7540 §6.6, §8.2).
                Err(FrameError::ProtocolError(
                    "PUSH_PROMISE not supported".into(),
                ))
            }
            // RFC 7540 §4.1: ignore and discard unknown frame types
            _ => Ok(None),
        }
    }

    /// Encode this frame (header + payload) into dst.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        match self {
            Frame::Data(f) => f.encode(dst),
            Frame::Headers(f) => f.encode(dst),
            Frame::Priority(f) => f.encode(dst),
            Frame::RstStream(f) => f.encode(dst),
            Frame::Settings(f) => f.encode(dst),
            Frame::Ping(f) => f.encode(dst),
            Frame::GoAway(f) => f.encode(dst),
            Frame::WindowUpdate(f) => f.encode(dst),
            Frame::Continuation(f) => f.encode(dst),
        }
    }

    /// The stream identifier for this frame, or `StreamId::ZERO` for
    /// connection frames.
    pub fn stream_id(&self) -> StreamId {
        match self {
            Frame::Data(f) => f.stream_id(),
            Frame::Headers(f) => f.stream_id(),
            Frame::Priority(f) => f.stream_id(),
            Frame::RstStream(f) => f.stream_id(),
            Frame::Settings(_) => StreamId::ZERO,
            Frame::Ping(_) => StreamId::ZERO,
            Frame::GoAway(_) => StreamId::ZERO,
            Frame::WindowUpdate(f) => f.stream_id(),
            Frame::Continuation(f) => f.stream_id(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_header_roundtrip() {
        let header = FrameHeader::new(FRAME_TYPE_DATA, 0x1, StreamId::new(1), 100);
        let encoded = header.encode();
        let decoded = FrameHeader::decode(&encoded);

        assert_eq!(decoded.length, 100);
        assert_eq!(decoded.frame_type, FRAME_TYPE_DATA);
        assert_eq!(decoded.flags, 0x1);
        assert_eq!(decoded.stream_id.value(), 1);
    }

    #[test]
    fn test_frame_header_max_length() {
        let header = FrameHeader::new(FRAME_TYPE_DATA, 0, StreamId::ZERO, MAX_FRAME_SIZE_UPPER);
        let encoded = header.encode();
        let decoded = FrameHeader::decode(&encoded);
        assert_eq!(decoded.length, MAX_FRAME_SIZE_UPPER);
    }

    #[test]
    fn test_unknown_frame_type_returns_none() {
        let unknown_type: u8 = 0xFF;
        let header = FrameHeader::new(unknown_type, 0, StreamId::ZERO, 4);
        let payload = Bytes::from(vec![0u8; 4]);
        let result = Frame::decode(header, payload).unwrap();
        assert!(
            result.is_none(),
            "unknown frame type should return Ok(None)"
        );
    }

    #[test]
    fn test_known_frame_type_returns_some() {
        // PING with 8-byte payload on stream 0
        let header = FrameHeader::new(FRAME_TYPE_PING, 0, StreamId::ZERO, 8);
        let payload = Bytes::from(vec![0u8; 8]);
        let result = Frame::decode(header, payload).unwrap();
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), Frame::Ping(_)));
    }

    #[test]
    fn test_ping_non_zero_stream_id_via_frame_decode() {
        let header = FrameHeader::new(FRAME_TYPE_PING, 0, StreamId::new(1), 8);
        let payload = Bytes::from(vec![0u8; 8]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_settings_non_zero_stream_id_via_frame_decode() {
        let header = FrameHeader::new(FRAME_TYPE_SETTINGS, 0, StreamId::new(1), 0);
        let payload = Bytes::new();
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_goaway_non_zero_stream_id_via_frame_decode() {
        let header = FrameHeader::new(FRAME_TYPE_GOAWAY, 0, StreamId::new(1), 8);
        let payload = Bytes::from(vec![0u8; 8]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    // --- Stream ID validation tests (RFC 7540 §6.x) ---

    #[test]
    fn test_data_zero_stream_id_rejected() {
        let header = FrameHeader::new(FRAME_TYPE_DATA, 0, StreamId::ZERO, 5);
        let payload = Bytes::from(vec![0u8; 5]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_headers_zero_stream_id_rejected() {
        let header = FrameHeader::new(FRAME_TYPE_HEADERS, 0x4, StreamId::ZERO, 0);
        let payload = Bytes::new();
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_priority_zero_stream_id_rejected() {
        let header = FrameHeader::new(FRAME_TYPE_PRIORITY, 0, StreamId::ZERO, 5);
        let payload = Bytes::from(vec![0u8; 5]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_rst_stream_zero_stream_id_rejected() {
        let header = FrameHeader::new(FRAME_TYPE_RST_STREAM, 0, StreamId::ZERO, 4);
        let payload = Bytes::from(vec![0u8; 4]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_continuation_zero_stream_id_rejected() {
        let header = FrameHeader::new(FRAME_TYPE_CONTINUATION, 0, StreamId::ZERO, 0);
        let payload = Bytes::new();
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_data_nonzero_stream_id_ok() {
        let header = FrameHeader::new(FRAME_TYPE_DATA, 0x1, StreamId::new(1), 5);
        let payload = Bytes::from(vec![0u8; 5]);
        let result = Frame::decode(header, payload);
        assert!(result.is_ok());
    }

    #[test]
    fn test_window_update_zero_stream_id_ok() {
        // WINDOW_UPDATE is valid on stream 0 (connection-level)
        let header = FrameHeader::new(FRAME_TYPE_WINDOW_UPDATE, 0, StreamId::ZERO, 4);
        let payload = Bytes::from(vec![0, 0, 0, 1]); // increment = 1
        let result = Frame::decode(header, payload);
        assert!(result.is_ok());
    }

    #[test]
    fn test_window_update_nonzero_stream_id_ok() {
        // WINDOW_UPDATE is also valid on non-zero streams
        let header = FrameHeader::new(FRAME_TYPE_WINDOW_UPDATE, 0, StreamId::new(3), 4);
        let payload = Bytes::from(vec![0, 0, 0, 1]);
        let result = Frame::decode(header, payload);
        assert!(result.is_ok());
    }

    #[test]
    fn test_push_promise_rejected() {
        // PUSH_PROMISE is not supported; Frame::decode returns ProtocolError
        let header = FrameHeader::new(FRAME_TYPE_PUSH_PROMISE, 0, StreamId::new(1), 4);
        let payload = Bytes::from(vec![0u8; 4]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            FrameError::ProtocolError(msg) => {
                assert!(msg.contains("PUSH_PROMISE"));
            }
            other => panic!("expected ProtocolError, got {:?}", other),
        }
    }

    #[test]
    fn test_push_promise_zero_stream_id_rejected() {
        // PUSH_PROMISE with stream_id=0 should fail stream_id validation first
        let header = FrameHeader::new(FRAME_TYPE_PUSH_PROMISE, 0, StreamId::ZERO, 4);
        let payload = Bytes::from(vec![0u8; 4]);
        let result = Frame::decode(header, payload);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_frame_any_stream_id_ok() {
        // Unknown frames: no stream_id constraint
        let header = FrameHeader::new(0xFE, 0, StreamId::new(42), 0);
        let result = Frame::decode(header, Bytes::new());
        assert!(result.unwrap().is_none());

        let header = FrameHeader::new(0xFE, 0, StreamId::ZERO, 0);
        let result = Frame::decode(header, Bytes::new());
        assert!(result.unwrap().is_none());
    }
}
