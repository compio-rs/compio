use bytes::Bytes;
use compio_buf::BufResult;
use compio_io::{AsyncRead, AsyncReadExt};

use crate::{
    error::{H2Error, Reason},
    frame::{
        DEFAULT_MAX_FRAME_SIZE, FRAME_HEADER_SIZE, FRAME_TYPE_CONTINUATION, Frame, FrameHeader,
        Headers, StreamId,
    },
};

/// Default maximum header list size (16 KB, matching HTTP/2 default
/// SETTINGS_MAX_HEADER_LIST_SIZE).
const DEFAULT_MAX_HEADER_LIST_SIZE: u32 = 16_384;

/// Minimum allowed continuation frame limit.
const MIN_CONTINUATION_LIMIT: usize = 5;

/// Tracks in-progress header block assembly across HEADERS + CONTINUATION
/// frames.
struct Partial {
    stream_id: StreamId,
    header_block: Vec<u8>,
    frame_count: usize,
    /// Original HEADERS frame flags (END_STREAM, PRIORITY, etc.).
    flags: u8,
    /// Priority fields from the original HEADERS frame.
    exclusive: bool,
    dependency: StreamId,
    weight: u8,
}

/// Reads HTTP/2 frames from an async reader.
///
/// Handles CONTINUATION frame assembly: when a HEADERS frame arrives without
/// END_HEADERS, subsequent CONTINUATION frames are accumulated and the complete
/// header block is returned as a single Headers frame.
pub struct FrameReader<IO> {
    io: IO,
    max_frame_size: u32,
    max_header_list_size: u32,
    max_continuation_frames: usize,
    partial: Option<Partial>,
}

impl<IO: AsyncRead> FrameReader<IO> {
    /// Create a new instance.
    pub fn new(io: IO) -> Self {
        let max_frame_size = DEFAULT_MAX_FRAME_SIZE;
        let max_header_list_size = DEFAULT_MAX_HEADER_LIST_SIZE;
        FrameReader {
            io,
            max_frame_size,
            max_header_list_size,
            max_continuation_frames: Self::compute_continuation_limit(
                max_header_list_size,
                max_frame_size,
            ),
            partial: None,
        }
    }

    /// Set the max frame size.
    pub fn set_max_frame_size(&mut self, size: u32) {
        self.max_frame_size = size;
        self.max_continuation_frames =
            Self::compute_continuation_limit(self.max_header_list_size, size);
    }

    /// Set the max header list size.
    pub fn set_max_header_list_size(&mut self, size: u32) {
        self.max_header_list_size = size;
        self.max_continuation_frames = Self::compute_continuation_limit(size, self.max_frame_size);
    }

    fn compute_continuation_limit(max_header_list_size: u32, max_frame_size: u32) -> usize {
        let ratio = (max_header_list_size as usize) / (max_frame_size as usize).max(1);
        (ratio.max(1) * 5 / 4).max(MIN_CONTINUATION_LIMIT)
    }

    /// Read a single HTTP/2 frame. Returns `Ok(None)` on clean EOF.
    ///
    /// When a HEADERS frame without END_HEADERS is received, this method
    /// continues reading CONTINUATION frames until END_HEADERS is set,
    /// then returns the reassembled Headers frame.
    ///
    /// Unknown frame types are silently discarded per RFC 7540 §4.1.
    pub async fn read_frame(&mut self) -> Result<Option<Frame>, H2Error> {
        loop {
            // Read 9-byte frame header
            let header_buf = Vec::with_capacity(FRAME_HEADER_SIZE);
            let BufResult(result, header_buf) = self.io.read_exact(header_buf).await;
            match result {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    if self.partial.is_some() {
                        return Err(H2Error::connection(Reason::ProtocolError));
                    }
                    return Ok(None);
                }
                Err(e) => return Err(H2Error::from(e)),
            }

            let header_arr: [u8; 9] = header_buf[..9]
                .try_into()
                .expect("read_exact guarantees 9 bytes");
            let header = FrameHeader::decode(&header_arr);

            if header.length > self.max_frame_size {
                return Err(H2Error::connection(Reason::FrameSizeError));
            }

            // Read payload
            let payload = if header.length > 0 {
                let payload_buf = Vec::with_capacity(header.length as usize);
                let BufResult(result, payload_buf) = self.io.read_exact(payload_buf).await;
                result.map_err(H2Error::from)?;
                Bytes::from(payload_buf)
            } else {
                Bytes::new()
            };

            // If we're in the middle of assembling a header block, only CONTINUATION is
            // valid
            if let Some(ref mut partial) = self.partial {
                if header.frame_type != FRAME_TYPE_CONTINUATION {
                    return Err(H2Error::connection(Reason::ProtocolError));
                }

                if header.stream_id != partial.stream_id {
                    return Err(H2Error::connection(Reason::ProtocolError));
                }

                partial.frame_count += 1;

                // Flood protection: too many CONTINUATION frames
                if partial.frame_count > self.max_continuation_frames {
                    return Err(H2Error::connection(Reason::EnhanceYourCalm));
                }

                partial.header_block.extend_from_slice(&payload);

                // Size protection: accumulated header block too large
                if partial.header_block.len() > self.max_header_list_size as usize {
                    return Err(H2Error::connection(Reason::CompressionError));
                }

                // Check END_HEADERS flag (0x4) on this CONTINUATION
                if header.flags & 0x4 != 0 {
                    let partial = self.partial.take().unwrap();
                    let combined_block = Bytes::from(partial.header_block);
                    let flags = partial.flags | 0x4; // Add END_HEADERS
                    let headers = Headers::from_parts(
                        partial.stream_id,
                        flags,
                        combined_block,
                        partial.exclusive,
                        partial.dependency,
                        partial.weight,
                    );
                    return Ok(Some(Frame::Headers(headers)));
                }

                continue;
            }

            // Normal frame decoding
            let frame = match Frame::decode(header, payload)? {
                Some(frame) => frame,
                None => continue, // Unknown frame type — skip and read next
            };

            // Check if this is a HEADERS frame without END_HEADERS
            if let Frame::Headers(ref h) = frame
                && !h.is_end_headers()
            {
                let partial = Partial {
                    stream_id: h.stream_id(),
                    header_block: h.header_block().to_vec(),
                    frame_count: 0,
                    flags: h.flags(),
                    exclusive: h.exclusive(),
                    dependency: h.dependency(),
                    weight: h.weight(),
                };

                if partial.header_block.len() > self.max_header_list_size as usize {
                    return Err(H2Error::connection(Reason::CompressionError));
                }

                self.partial = Some(partial);
                continue;
            }

            return Ok(Some(frame));
        }
    }

    /// Read raw bytes (used for connection preface).
    pub async fn read_exact_bytes(&mut self, len: usize) -> Result<Vec<u8>, H2Error> {
        let buf = Vec::with_capacity(len);
        let BufResult(result, buf) = self.io.read_exact(buf).await;
        result.map_err(H2Error::from)?;
        Ok(buf)
    }

    /// Read and discard any data sitting in the kernel receive buffer.
    ///
    /// A single bounded read clears leftover bytes so that `close()` on the
    /// socket does not see unread data and send RST, which would discard
    /// outbound frames (like GOAWAY) that the peer has not yet read.
    pub async fn clear_recv_buffer(&mut self) {
        let buf = vec![0u8; 4096];
        let _ = self.io.read(buf).await;
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::frame::{Continuation, Data, StreamId};

    #[compio_macros::test]
    async fn test_read_data_frame() {
        let mut data_frame = Data::new(StreamId::new(1), Bytes::from_static(b"hello"));
        data_frame.set_end_stream();

        let mut buf = Vec::new();
        data_frame.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        let frame = reader.read_frame().await.unwrap().unwrap();
        match frame {
            Frame::Data(d) => {
                assert_eq!(d.stream_id().value(), 1);
                assert!(d.is_end_stream());
                assert_eq!(d.payload().as_ref(), b"hello");
            }
            _ => panic!("expected Data frame"),
        }
    }

    #[compio_macros::test]
    async fn test_read_eof() {
        let cursor = Cursor::new(Vec::new());
        let mut reader = FrameReader::new(cursor);
        assert!(reader.read_frame().await.unwrap().is_none());
    }

    #[compio_macros::test]
    async fn test_read_unknown_frame_type_skipped() {
        use crate::frame::FrameHeader;

        let mut buf = Vec::new();

        // First: an unknown frame type (0xFE) with 4-byte payload
        let unknown_header = FrameHeader::new(0xFE, 0, StreamId::new(0), 4);
        buf.extend_from_slice(&unknown_header.encode());
        buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        // Second: a valid DATA frame
        let mut data_frame = Data::new(StreamId::new(1), Bytes::from_static(b"ok"));
        data_frame.set_end_stream();
        data_frame.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        // Should skip the unknown frame and return the DATA frame
        let frame = reader.read_frame().await.unwrap().unwrap();
        match frame {
            Frame::Data(d) => {
                assert_eq!(d.stream_id().value(), 1);
                assert_eq!(d.payload().as_ref(), b"ok");
            }
            _ => panic!("expected Data frame after skipping unknown frame"),
        }
    }

    #[compio_macros::test]
    async fn test_read_frame_too_large() {
        let header = FrameHeader::new(0x0, 0, StreamId::new(1), DEFAULT_MAX_FRAME_SIZE + 1);
        let encoded = header.encode();

        let cursor = Cursor::new(encoded.to_vec());
        let mut reader = FrameReader::new(cursor);

        let result = reader.read_frame().await;
        assert!(result.is_err());
    }

    /// HEADERS + CONTINUATION frames are assembled into a single Headers frame.
    #[compio_macros::test]
    async fn test_continuation_assembly() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        // HEADERS with END_STREAM but without END_HEADERS
        let headers = Headers::from_parts(
            stream_id,
            0x01, // END_STREAM only
            Bytes::from_static(b"\x82\x86"),
            false,
            StreamId::ZERO,
            16,
        );
        headers.encode(&mut buf);

        // CONTINUATION without END_HEADERS
        let cont1 = Continuation::new(stream_id, Bytes::from_static(b"\x84"));
        cont1.encode(&mut buf);

        // CONTINUATION with END_HEADERS
        let mut cont2 = Continuation::new(stream_id, Bytes::from_static(b"\x87"));
        cont2.set_end_headers();
        cont2.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        let frame = reader.read_frame().await.unwrap().unwrap();
        match frame {
            Frame::Headers(h) => {
                assert_eq!(h.stream_id().value(), 1);
                assert!(h.is_end_stream());
                assert!(h.is_end_headers());
                assert_eq!(h.header_block().as_ref(), b"\x82\x86\x84\x87");
            }
            _ => panic!("expected reassembled Headers frame"),
        }
    }

    /// Non-CONTINUATION frame during assembly causes PROTOCOL_ERROR.
    #[compio_macros::test]
    async fn test_non_continuation_during_assembly_is_protocol_error() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        let headers = Headers::from_parts(
            stream_id,
            0x00,
            Bytes::from_static(b"\x82"),
            false,
            StreamId::ZERO,
            16,
        );
        headers.encode(&mut buf);

        // DATA frame instead of CONTINUATION
        let data = Data::new(stream_id, Bytes::from_static(b"oops"));
        data.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        match reader.read_frame().await {
            Err(H2Error::ConnectionError {
                reason: Reason::ProtocolError,
                ..
            }) => {}
            other => panic!("expected ProtocolError, got {:?}", other),
        }
    }

    /// CONTINUATION on wrong stream during assembly causes PROTOCOL_ERROR.
    #[compio_macros::test]
    async fn test_continuation_wrong_stream_is_protocol_error() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        let headers = Headers::from_parts(
            stream_id,
            0x00,
            Bytes::from_static(b"\x82"),
            false,
            StreamId::ZERO,
            16,
        );
        headers.encode(&mut buf);

        let mut cont = Continuation::new(StreamId::new(3), Bytes::from_static(b"\x84"));
        cont.set_end_headers();
        cont.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        match reader.read_frame().await {
            Err(H2Error::ConnectionError {
                reason: Reason::ProtocolError,
                ..
            }) => {}
            other => panic!("expected ProtocolError, got {:?}", other),
        }
    }

    /// Too many CONTINUATION frames triggers ENHANCE_YOUR_CALM.
    #[compio_macros::test]
    async fn test_continuation_flood_protection() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        let headers = Headers::from_parts(
            stream_id,
            0x00,
            Bytes::from_static(b"\x82"),
            false,
            StreamId::ZERO,
            16,
        );
        headers.encode(&mut buf);

        // Default limit is 5. Send 10 CONTINUATION frames without END_HEADERS.
        for _ in 0..10 {
            let cont = Continuation::new(stream_id, Bytes::from_static(b"\x84"));
            cont.encode(&mut buf);
        }

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        match reader.read_frame().await {
            Err(H2Error::ConnectionError {
                reason: Reason::EnhanceYourCalm,
                ..
            }) => {}
            other => panic!("expected EnhanceYourCalm, got {:?}", other),
        }
    }

    /// Accumulated header block exceeding max_header_list_size triggers
    /// COMPRESSION_ERROR.
    #[compio_macros::test]
    async fn test_continuation_size_limit() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        let headers = Headers::from_parts(
            stream_id,
            0x00,
            Bytes::from_static(b"\x82"),
            false,
            StreamId::ZERO,
            16,
        );
        headers.encode(&mut buf);

        // Large CONTINUATION payload exceeding 16KB default
        let large_payload = vec![0x84u8; 20_000];
        let cont = Continuation::new(stream_id, Bytes::from(large_payload));
        cont.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);
        // Increase max_frame_size so the large frame isn't rejected at the frame level
        reader.set_max_frame_size(32_768);

        match reader.read_frame().await {
            Err(H2Error::ConnectionError {
                reason: Reason::CompressionError,
                ..
            }) => {}
            other => panic!("expected CompressionError, got {:?}", other),
        }
    }

    /// EOF during CONTINUATION assembly is a protocol error.
    #[compio_macros::test]
    async fn test_eof_during_continuation_assembly() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        let headers = Headers::from_parts(
            stream_id,
            0x00,
            Bytes::from_static(b"\x82"),
            false,
            StreamId::ZERO,
            16,
        );
        headers.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        match reader.read_frame().await {
            Err(H2Error::ConnectionError {
                reason: Reason::ProtocolError,
                ..
            }) => {}
            other => panic!(
                "expected ProtocolError on EOF during assembly, got {:?}",
                other
            ),
        }
    }

    /// HEADERS with END_HEADERS passes through without assembly.
    #[compio_macros::test]
    async fn test_headers_with_end_headers_no_assembly() {
        let stream_id = StreamId::new(1);
        let mut buf = Vec::new();

        let mut headers = Headers::new(stream_id, Bytes::from_static(b"\x82\x86"));
        headers.set_end_stream();
        headers.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        let frame = reader.read_frame().await.unwrap().unwrap();
        match frame {
            Frame::Headers(h) => {
                assert_eq!(h.stream_id().value(), 1);
                assert!(h.is_end_stream());
                assert!(h.is_end_headers());
                assert_eq!(h.header_block().as_ref(), b"\x82\x86");
            }
            _ => panic!("expected Headers frame"),
        }
    }

    /// Roundtrip: encode a DATA frame, read it back.
    #[compio_macros::test]
    async fn test_frame_encode_decode_roundtrip() {
        let mut data_frame = Data::new(StreamId::new(1), Bytes::from_static(b"hello"));
        data_frame.set_end_stream();
        let frame = Frame::Data(data_frame);

        let mut buf = Vec::new();
        frame.encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);
        let read_frame = reader.read_frame().await.unwrap().unwrap();

        match read_frame {
            Frame::Data(d) => {
                assert_eq!(d.stream_id().value(), 1);
                assert!(d.is_end_stream());
                assert_eq!(d.payload().as_ref(), b"hello");
            }
            _ => panic!("expected Data frame"),
        }
    }

    /// Roundtrip: encode multiple control frames, read them all back.
    #[compio_macros::test]
    async fn test_multiple_frames_roundtrip() {
        use crate::frame::{Ping, Settings, WindowUpdate};

        let mut buf = Vec::new();
        Frame::Ping(Ping::new([1, 2, 3, 4, 5, 6, 7, 8])).encode(&mut buf);
        Frame::Settings(Settings::ack()).encode(&mut buf);
        Frame::WindowUpdate(WindowUpdate::new(StreamId::ZERO, 1000)).encode(&mut buf);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);

        let f1 = reader.read_frame().await.unwrap().unwrap();
        assert!(matches!(f1, Frame::Ping(_)));
        let f2 = reader.read_frame().await.unwrap().unwrap();
        assert!(matches!(f2, Frame::Settings(_)));
        let f3 = reader.read_frame().await.unwrap().unwrap();
        assert!(matches!(f3, Frame::WindowUpdate(_)));
    }

    /// Roundtrip: encode a DATA frame manually via FrameHeader + payload.
    #[compio_macros::test]
    async fn test_manual_data_frame_roundtrip() {
        let stream_id = StreamId::new(1);
        let payload = Bytes::from_static(b"hello world");
        let len = payload.len() as u32;

        let header = crate::frame::FrameHeader::new(0x0, 0x1, stream_id, len);
        let mut buf = Vec::new();
        buf.extend_from_slice(&header.encode());
        buf.extend_from_slice(&payload);

        let cursor = Cursor::new(buf);
        let mut reader = FrameReader::new(cursor);
        let frame = reader.read_frame().await.unwrap().unwrap();
        match frame {
            Frame::Data(d) => {
                assert_eq!(d.stream_id().value(), 1);
                assert!(d.is_end_stream());
                assert_eq!(d.payload().as_ref(), b"hello world");
            }
            _ => panic!("expected Data frame"),
        }
    }

    /// clear_recv_buffer discards pending data without blocking.
    #[compio_macros::test]
    async fn test_clear_recv_buffer_discards_data() {
        let data = b"leftover bytes in kernel buffer";
        let cursor = Cursor::new(data.to_vec());
        let mut reader = FrameReader::new(cursor);
        reader.clear_recv_buffer().await;
        // After clearing, a subsequent read should return 0 (EOF).
        let buf = vec![0u8; 64];
        let BufResult(result, _) = reader.io.read(buf).await;
        assert_eq!(result.unwrap(), 0);
    }
}
