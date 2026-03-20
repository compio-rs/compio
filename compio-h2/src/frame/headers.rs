use bytes::Bytes;

use super::{FRAME_TYPE_HEADERS, FrameHeader, stream_id::StreamId};
use crate::error::FrameError;

const FLAG_END_STREAM: u8 = 0x1;
const FLAG_END_HEADERS: u8 = 0x4;
const FLAG_PADDED: u8 = 0x8;
const FLAG_PRIORITY: u8 = 0x20;

/// HEADERS frame (type=0x1).
#[derive(Debug, Clone)]
pub struct Headers {
    stream_id: StreamId,
    header_block: Bytes,
    flags: u8,
    // Priority fields (present if PRIORITY flag is set)
    exclusive: bool,
    dependency: StreamId,
    weight: u8,
}

impl Headers {
    /// Create a new HEADERS frame with END_HEADERS set by default.
    pub fn new(stream_id: StreamId, header_block: Bytes) -> Self {
        Headers {
            stream_id,
            header_block,
            flags: FLAG_END_HEADERS,
            exclusive: false,
            dependency: StreamId::ZERO,
            weight: 16,
        }
    }

    /// The stream identifier for this HEADERS frame.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// A reference to the encoded HPACK header block fragment.
    pub fn header_block(&self) -> &Bytes {
        &self.header_block
    }

    /// Consume this frame and return the header block fragment.
    pub fn into_header_block(self) -> Bytes {
        self.header_block
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

    /// Whether the END_HEADERS flag (0x4) is set.
    pub fn is_end_headers(&self) -> bool {
        self.flags & FLAG_END_HEADERS != 0
    }

    /// Set the END_HEADERS flag, indicating the header block is complete.
    pub fn set_end_headers(&mut self) {
        self.flags |= FLAG_END_HEADERS;
    }

    /// Clears the END_HEADERS flag (used when splitting into CONTINUATION
    /// frames).
    pub fn clear_end_headers(&mut self) {
        self.flags &= !FLAG_END_HEADERS;
    }

    /// Whether the PRIORITY flag (0x20) is set.
    pub fn has_priority(&self) -> bool {
        self.flags & FLAG_PRIORITY != 0
    }

    /// Set the PRIORITY flag and associated stream dependency fields.
    pub fn set_priority(&mut self, exclusive: bool, dependency: StreamId, weight: u8) {
        self.flags |= FLAG_PRIORITY;
        self.exclusive = exclusive;
        self.dependency = dependency;
        self.weight = weight;
    }

    /// Whether this stream has an exclusive dependency.
    pub fn exclusive(&self) -> bool {
        self.exclusive
    }

    /// The stream dependency identifier.
    pub fn dependency(&self) -> StreamId {
        self.dependency
    }

    /// The priority weight (1-256, encoded as 0-255).
    pub fn weight(&self) -> u8 {
        self.weight
    }

    /// The raw flags byte.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// Construct a HEADERS frame from pre-decoded parts (used for CONTINUATION
    /// reassembly).
    pub(crate) fn from_parts(
        stream_id: StreamId,
        flags: u8,
        header_block: Bytes,
        exclusive: bool,
        dependency: StreamId,
        weight: u8,
    ) -> Self {
        Headers {
            stream_id,
            header_block,
            flags,
            exclusive,
            dependency,
            weight,
        }
    }

    /// Decode a HEADERS frame from payload bytes.
    pub fn decode(stream_id: StreamId, flags: u8, payload: Bytes) -> Result<Self, FrameError> {
        if stream_id.is_zero() {
            return Err(FrameError::InvalidStreamId(
                "HEADERS frame with stream ID 0".into(),
            ));
        }

        let mut offset = 0;
        let data = &payload[..];

        // Handle padding
        let pad_len = if flags & FLAG_PADDED != 0 {
            if data.is_empty() {
                return Err(FrameError::InvalidPadding(
                    "padded HEADERS with no pad length".into(),
                ));
            }
            let p = data[0] as usize;
            offset += 1;
            p
        } else {
            0
        };

        // Handle priority
        let (exclusive, dependency, weight) = if flags & FLAG_PRIORITY != 0 {
            if data.len() < offset + 5 {
                return Err(FrameError::InvalidPayload(
                    "HEADERS with PRIORITY flag but insufficient data".into(),
                ));
            }
            let dep_raw = ((data[offset] as u32) << 24)
                | ((data[offset + 1] as u32) << 16)
                | ((data[offset + 2] as u32) << 8)
                | (data[offset + 3] as u32);
            let exclusive = dep_raw & 0x8000_0000 != 0;
            let dependency = StreamId::new(dep_raw & 0x7FFF_FFFF);
            let weight = data[offset + 4];
            offset += 5;
            if dependency == stream_id {
                return Err(FrameError::ProtocolError(format!(
                    "HEADERS frame with self-dependency on stream {}",
                    stream_id.value()
                )));
            }
            (exclusive, dependency, weight)
        } else {
            (false, StreamId::ZERO, 16)
        };

        if pad_len > 0 && offset + pad_len > payload.len() {
            return Err(FrameError::InvalidPadding(
                "pad length exceeds frame payload".into(),
            ));
        }

        let end = payload.len() - pad_len;
        let header_block = payload.slice(offset..end);

        Ok(Headers {
            stream_id,
            header_block,
            flags,
            exclusive,
            dependency,
            weight,
        })
    }

    /// Encode this HEADERS frame.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        let mut payload_len = self.header_block.len();
        if self.has_priority() {
            payload_len += 5;
        }

        dst.extend_from_slice(
            &FrameHeader::new(
                FRAME_TYPE_HEADERS,
                self.flags,
                self.stream_id,
                payload_len as u32,
            )
            .encode(),
        );

        // Priority fields
        if self.has_priority() {
            let dep = self.dependency.value() | if self.exclusive { 0x8000_0000 } else { 0 };
            dst.push((dep >> 24) as u8);
            dst.push((dep >> 16) as u8);
            dst.push((dep >> 8) as u8);
            dst.push(dep as u8);
            dst.push(self.weight);
        }

        dst.extend_from_slice(&self.header_block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headers_roundtrip() {
        let mut frame = Headers::new(StreamId::new(1), Bytes::from_static(b"\x82\x86"));
        frame.set_end_stream();

        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let len = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        let flags = buf[4];
        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);

        let decoded = Headers::decode(
            StreamId::new(sid),
            flags,
            Bytes::copy_from_slice(&buf[9..9 + len as usize]),
        )
        .unwrap();
        assert_eq!(decoded.stream_id().value(), 1);
        assert!(decoded.is_end_stream());
        assert!(decoded.is_end_headers());
        assert_eq!(decoded.header_block().as_ref(), b"\x82\x86");
    }

    #[test]
    fn test_headers_self_dependency_rejected() {
        // PRIORITY flag set with dependency == stream_id → ProtocolError
        let mut payload = Vec::new();
        // dependency = stream 5 (same as frame stream_id), exclusive bit clear
        let dep: u32 = 5;
        payload.push((dep >> 24) as u8);
        payload.push((dep >> 16) as u8);
        payload.push((dep >> 8) as u8);
        payload.push(dep as u8);
        payload.push(16); // weight
        payload.extend_from_slice(b"\x82"); // header block

        let flags = FLAG_END_HEADERS | FLAG_PRIORITY;
        let result = Headers::decode(StreamId::new(5), flags, Bytes::from(payload));
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            FrameError::ProtocolError(msg) => assert!(msg.contains("self-dependency")),
            other => panic!("expected ProtocolError, got {:?}", other),
        }
    }

    #[test]
    fn test_headers_different_dependency_ok() {
        // PRIORITY flag with dependency != stream_id → ok
        let mut payload = Vec::new();
        let dep: u32 = 1;
        payload.push((dep >> 24) as u8);
        payload.push((dep >> 16) as u8);
        payload.push((dep >> 8) as u8);
        payload.push(dep as u8);
        payload.push(16); // weight
        payload.extend_from_slice(b"\x82"); // header block

        let flags = FLAG_END_HEADERS | FLAG_PRIORITY;
        let result = Headers::decode(StreamId::new(5), flags, Bytes::from(payload));
        assert!(result.is_ok());
    }

    #[test]
    fn test_headers_with_priority() {
        let mut frame = Headers::new(StreamId::new(3), Bytes::from_static(b"\x82"));
        frame.set_priority(true, StreamId::new(1), 255);

        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let len = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        let flags = buf[4];
        let sid = ((buf[5] as u32) << 24)
            | ((buf[6] as u32) << 16)
            | ((buf[7] as u32) << 8)
            | (buf[8] as u32);

        let decoded = Headers::decode(
            StreamId::new(sid),
            flags,
            Bytes::copy_from_slice(&buf[9..9 + len as usize]),
        )
        .unwrap();
        assert!(decoded.has_priority());
        assert!(decoded.exclusive());
        assert_eq!(decoded.dependency().value(), 1);
        assert_eq!(decoded.weight(), 255);
        assert_eq!(decoded.header_block().as_ref(), b"\x82");
    }
}
