use super::{FRAME_TYPE_SETTINGS, FrameHeader};
use crate::error::FrameError;

const FLAG_ACK: u8 = 0x1;

// Settings identifiers
const HEADER_TABLE_SIZE: u16 = 0x1;
const ENABLE_PUSH: u16 = 0x2;
const MAX_CONCURRENT_STREAMS: u16 = 0x3;
const INITIAL_WINDOW_SIZE: u16 = 0x4;
const MAX_FRAME_SIZE: u16 = 0x5;
const MAX_HEADER_LIST_SIZE: u16 = 0x6;

/// SETTINGS frame (type=0x4).
#[derive(Debug, Clone, Default)]
pub struct Settings {
    flags: u8,
    header_table_size: Option<u32>,
    enable_push: Option<bool>,
    max_concurrent_streams: Option<u32>,
    initial_window_size: Option<u32>,
    max_frame_size: Option<u32>,
    max_header_list_size: Option<u32>,
}

impl Settings {
    /// Create a new empty SETTINGS frame with no parameters set.
    pub fn new() -> Self {
        Settings::default()
    }

    /// Create a SETTINGS acknowledgement frame (RFC 9113 Section 6.5).
    pub fn ack() -> Self {
        Settings {
            flags: FLAG_ACK,
            ..Default::default()
        }
    }

    /// Whether the ACK flag (0x1) is set.
    pub fn is_ack(&self) -> bool {
        self.flags & FLAG_ACK != 0
    }

    /// The raw flags byte.
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// The HEADER_TABLE_SIZE setting (0x1), if present.
    pub fn header_table_size(&self) -> Option<u32> {
        self.header_table_size
    }

    /// Set the HEADER_TABLE_SIZE parameter (HPACK dynamic table size in
    /// bytes).
    pub fn set_header_table_size(&mut self, val: u32) {
        self.header_table_size = Some(val);
    }

    /// The ENABLE_PUSH setting (0x2), if present.
    pub fn enable_push(&self) -> Option<bool> {
        self.enable_push
    }

    /// Set the ENABLE_PUSH parameter (must be 0 or 1 per RFC 9113 Section
    /// 6.5.2).
    pub fn set_enable_push(&mut self, val: bool) {
        self.enable_push = Some(val);
    }

    /// The MAX_CONCURRENT_STREAMS setting (0x3), if present.
    pub fn max_concurrent_streams(&self) -> Option<u32> {
        self.max_concurrent_streams
    }

    /// Set the MAX_CONCURRENT_STREAMS parameter.
    pub fn set_max_concurrent_streams(&mut self, val: u32) {
        self.max_concurrent_streams = Some(val);
    }

    /// The INITIAL_WINDOW_SIZE setting (0x4), if present.
    pub fn initial_window_size(&self) -> Option<u32> {
        self.initial_window_size
    }

    /// Set the INITIAL_WINDOW_SIZE parameter (max 2^31-1 per RFC 9113 Section
    /// 6.5.2).
    pub fn set_initial_window_size(&mut self, val: u32) {
        self.initial_window_size = Some(val);
    }

    /// The MAX_FRAME_SIZE setting (0x5), if present.
    pub fn max_frame_size(&self) -> Option<u32> {
        self.max_frame_size
    }

    /// Set the MAX_FRAME_SIZE parameter (must be between 2^14 and 2^24-1).
    pub fn set_max_frame_size(&mut self, val: u32) {
        self.max_frame_size = Some(val);
    }

    /// The MAX_HEADER_LIST_SIZE setting (0x6), if present.
    pub fn max_header_list_size(&self) -> Option<u32> {
        self.max_header_list_size
    }

    /// Set the MAX_HEADER_LIST_SIZE parameter (advisory limit on header list
    /// size).
    pub fn set_max_header_list_size(&mut self, val: u32) {
        self.max_header_list_size = Some(val);
    }

    /// Decode a SETTINGS frame from payload bytes. Per RFC 7540 §6.5,
    /// stream_id MUST be 0.
    pub fn decode(
        stream_id: super::stream_id::StreamId,
        flags: u8,
        payload: &[u8],
    ) -> Result<Self, FrameError> {
        if !stream_id.is_zero() {
            return Err(FrameError::ProtocolError(
                "SETTINGS frame with non-zero stream ID (PROTOCOL_ERROR)".into(),
            ));
        }
        if flags & FLAG_ACK != 0 {
            if !payload.is_empty() {
                return Err(FrameError::InvalidFrameSize(
                    "SETTINGS ACK with non-empty payload".into(),
                ));
            }
            return Ok(Settings::ack());
        }

        if !payload.len().is_multiple_of(6) {
            return Err(FrameError::InvalidFrameSize(
                "SETTINGS payload length not a multiple of 6".into(),
            ));
        }

        let mut settings = Settings::new();
        settings.flags = flags;

        let mut i = 0;
        while i < payload.len() {
            let id = ((payload[i] as u16) << 8) | (payload[i + 1] as u16);
            let value = ((payload[i + 2] as u32) << 24)
                | ((payload[i + 3] as u32) << 16)
                | ((payload[i + 4] as u32) << 8)
                | (payload[i + 5] as u32);
            i += 6;

            match id {
                HEADER_TABLE_SIZE => settings.header_table_size = Some(value),
                ENABLE_PUSH => {
                    if value > 1 {
                        return Err(FrameError::InvalidPayload(
                            "ENABLE_PUSH must be 0 or 1".into(),
                        ));
                    }
                    settings.enable_push = Some(value == 1);
                }
                MAX_CONCURRENT_STREAMS => settings.max_concurrent_streams = Some(value),
                INITIAL_WINDOW_SIZE => {
                    if value > 0x7FFF_FFFF {
                        return Err(FrameError::FlowControlError(
                            "INITIAL_WINDOW_SIZE exceeds 2^31-1".into(),
                        ));
                    }
                    settings.initial_window_size = Some(value);
                }
                MAX_FRAME_SIZE => {
                    if !(16_384..=16_777_215).contains(&value) {
                        return Err(FrameError::InvalidPayload(
                            "MAX_FRAME_SIZE out of range".into(),
                        ));
                    }
                    settings.max_frame_size = Some(value);
                }
                MAX_HEADER_LIST_SIZE => settings.max_header_list_size = Some(value),
                _ => {
                    // Unknown settings are ignored per RFC 7540 Section 6.5.2
                }
            }
        }

        Ok(settings)
    }

    /// Encode this SETTINGS frame.
    pub fn encode(&self, dst: &mut Vec<u8>) {
        let mut payload = Vec::new();

        if !self.is_ack() {
            if let Some(v) = self.header_table_size {
                encode_setting(&mut payload, HEADER_TABLE_SIZE, v);
            }
            if let Some(v) = self.enable_push {
                encode_setting(&mut payload, ENABLE_PUSH, v as u32);
            }
            if let Some(v) = self.max_concurrent_streams {
                encode_setting(&mut payload, MAX_CONCURRENT_STREAMS, v);
            }
            if let Some(v) = self.initial_window_size {
                encode_setting(&mut payload, INITIAL_WINDOW_SIZE, v);
            }
            if let Some(v) = self.max_frame_size {
                encode_setting(&mut payload, MAX_FRAME_SIZE, v);
            }
            if let Some(v) = self.max_header_list_size {
                encode_setting(&mut payload, MAX_HEADER_LIST_SIZE, v);
            }
        }

        let len = payload.len() as u32;
        dst.extend_from_slice(
            &FrameHeader::new(
                FRAME_TYPE_SETTINGS,
                self.flags,
                super::stream_id::StreamId::ZERO,
                len,
            )
            .encode(),
        );
        dst.extend_from_slice(&payload);
    }
}

fn encode_setting(dst: &mut Vec<u8>, id: u16, value: u32) {
    dst.push((id >> 8) as u8);
    dst.push(id as u8);
    dst.push((value >> 24) as u8);
    dst.push((value >> 16) as u8);
    dst.push((value >> 8) as u8);
    dst.push(value as u8);
}

#[cfg(test)]
mod tests {
    use super::{super::stream_id::StreamId, *};

    #[test]
    fn test_settings_roundtrip() {
        let mut settings = Settings::new();
        settings.set_header_table_size(4096);
        settings.set_max_concurrent_streams(100);
        settings.set_initial_window_size(65535);
        settings.set_max_frame_size(16384);

        let mut buf = Vec::new();
        settings.encode(&mut buf);

        let flags = buf[4];
        let payload = &buf[9..];
        let decoded = Settings::decode(StreamId::ZERO, flags, payload).unwrap();

        assert_eq!(decoded.header_table_size(), Some(4096));
        assert_eq!(decoded.max_concurrent_streams(), Some(100));
        assert_eq!(decoded.initial_window_size(), Some(65535));
        assert_eq!(decoded.max_frame_size(), Some(16384));
    }

    #[test]
    fn test_settings_ack() {
        let settings = Settings::ack();
        let mut buf = Vec::new();
        settings.encode(&mut buf);

        assert_eq!(buf.len(), 9); // Header only, no payload
        let flags = buf[4];
        let payload = &buf[9..];
        let decoded = Settings::decode(StreamId::ZERO, flags, payload).unwrap();
        assert!(decoded.is_ack());
    }

    #[test]
    fn test_settings_max_frame_size_boundaries() {
        // 16383 is below the RFC minimum of 16384 — must be rejected
        let mut payload_low = Vec::new();
        payload_low.extend_from_slice(&0x5u16.to_be_bytes()); // SETTINGS_MAX_FRAME_SIZE
        payload_low.extend_from_slice(&16383u32.to_be_bytes());
        let result = Settings::decode(StreamId::ZERO, 0, &payload_low);
        assert!(result.is_err(), "MAX_FRAME_SIZE=16383 should be rejected");

        // 16384 is the exact minimum — must be accepted
        let mut payload_ok = Vec::new();
        payload_ok.extend_from_slice(&0x5u16.to_be_bytes());
        payload_ok.extend_from_slice(&16384u32.to_be_bytes());
        let result = Settings::decode(StreamId::ZERO, 0, &payload_ok);
        assert!(result.is_ok(), "MAX_FRAME_SIZE=16384 should be accepted");

        // 16777215 (2^24-1) is the maximum — must be accepted
        let mut payload_max = Vec::new();
        payload_max.extend_from_slice(&0x5u16.to_be_bytes());
        payload_max.extend_from_slice(&16_777_215u32.to_be_bytes());
        let result = Settings::decode(StreamId::ZERO, 0, &payload_max);
        assert!(result.is_ok(), "MAX_FRAME_SIZE=16777215 should be accepted");

        // 16777216 (2^24) exceeds the maximum — must be rejected
        let mut payload_over = Vec::new();
        payload_over.extend_from_slice(&0x5u16.to_be_bytes());
        payload_over.extend_from_slice(&16_777_216u32.to_be_bytes());
        let result = Settings::decode(StreamId::ZERO, 0, &payload_over);
        assert!(
            result.is_err(),
            "MAX_FRAME_SIZE=16777216 should be rejected"
        );
    }

    #[test]
    fn test_settings_non_zero_stream_id() {
        let result = Settings::decode(StreamId::new(1), 0, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("PROTOCOL_ERROR"));
    }
}
