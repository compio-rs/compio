use std::fmt;

use bytes::Bytes;

/// HTTP/2 error codes as defined in RFC 7540 Section 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// No error occurred (0x0). Used for graceful shutdown.
    NoError,
    /// Generic protocol error detected (0x1).
    ProtocolError,
    /// Implementation-specific internal error (0x2).
    InternalError,
    /// Flow control limits were exceeded (0x3).
    FlowControlError,
    /// Settings ACK not received in time (0x4).
    SettingsTimeout,
    /// Frame received on a half-closed or closed stream (0x5).
    StreamClosed,
    /// Frame size was invalid (0x6).
    FrameSizeError,
    /// Endpoint refused the stream before processing (0x7).
    RefusedStream,
    /// Stream is no longer needed (0x8).
    Cancel,
    /// HPACK compression context could not be maintained (0x9).
    CompressionError,
    /// TCP connection established in response to CONNECT was reset or
    /// abnormally closed (0xa).
    ConnectError,
    /// Endpoint detected excessive load (0xb).
    EnhanceYourCalm,
    /// Transport-layer security negotiation failed or is inadequate (0xc).
    InadequateSecurity,
    /// Endpoint requires HTTP/1.1 instead of HTTP/2 (0xd).
    Http11Required,
    /// Unknown error code not defined in RFC 7540. The raw value is preserved.
    Unknown(u32),
}

impl Reason {
    /// Convert a raw u32 error code to a `Reason`.
    /// Unknown codes are preserved as `Reason::Unknown(code)`.
    pub fn from_u32(n: u32) -> Reason {
        match n {
            0x0 => Reason::NoError,
            0x1 => Reason::ProtocolError,
            0x2 => Reason::InternalError,
            0x3 => Reason::FlowControlError,
            0x4 => Reason::SettingsTimeout,
            0x5 => Reason::StreamClosed,
            0x6 => Reason::FrameSizeError,
            0x7 => Reason::RefusedStream,
            0x8 => Reason::Cancel,
            0x9 => Reason::CompressionError,
            0xa => Reason::ConnectError,
            0xb => Reason::EnhanceYourCalm,
            0xc => Reason::InadequateSecurity,
            0xd => Reason::Http11Required,
            other => Reason::Unknown(other),
        }
    }

    /// Convert this Reason to its wire representation (u32 error code).
    pub fn to_u32(self) -> u32 {
        match self {
            Reason::NoError => 0x0,
            Reason::ProtocolError => 0x1,
            Reason::InternalError => 0x2,
            Reason::FlowControlError => 0x3,
            Reason::SettingsTimeout => 0x4,
            Reason::StreamClosed => 0x5,
            Reason::FrameSizeError => 0x6,
            Reason::RefusedStream => 0x7,
            Reason::Cancel => 0x8,
            Reason::CompressionError => 0x9,
            Reason::ConnectError => 0xa,
            Reason::EnhanceYourCalm => 0xb,
            Reason::InadequateSecurity => 0xc,
            Reason::Http11Required => 0xd,
            Reason::Unknown(code) => code,
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::NoError => f.write_str("no error"),
            Reason::ProtocolError => f.write_str("protocol error"),
            Reason::InternalError => f.write_str("internal error"),
            Reason::FlowControlError => f.write_str("flow control error"),
            Reason::SettingsTimeout => f.write_str("settings timeout"),
            Reason::StreamClosed => f.write_str("stream closed"),
            Reason::FrameSizeError => f.write_str("frame size error"),
            Reason::RefusedStream => f.write_str("refused stream"),
            Reason::Cancel => f.write_str("cancel"),
            Reason::CompressionError => f.write_str("compression error"),
            Reason::ConnectError => f.write_str("connect error"),
            Reason::EnhanceYourCalm => f.write_str("enhance your calm"),
            Reason::InadequateSecurity => f.write_str("inadequate security"),
            Reason::Http11Required => f.write_str("HTTP/1.1 required"),
            Reason::Unknown(code) => write!(f, "unknown error code (0x{:x})", code),
        }
    }
}

/// Errors that occur when decoding HTTP/2 frame headers and payloads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The frame size is invalid for the frame type.
    #[error("invalid frame size: {0}")]
    InvalidFrameSize(String),

    /// The stream ID is invalid for the frame type.
    #[error("invalid stream ID: {0}")]
    InvalidStreamId(String),

    /// The padding length is invalid.
    #[error("invalid padding: {0}")]
    InvalidPadding(String),

    /// The payload content is invalid.
    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    /// A protocol-level error in the frame.
    #[error("frame protocol error: {0}")]
    ProtocolError(String),

    /// A flow-control error in the frame (e.g., INITIAL_WINDOW_SIZE > 2^31-1).
    #[error("flow control error: {0}")]
    FlowControlError(String),
}

/// Errors that occur during HPACK header compression/decompression.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HpackError {
    /// The integer encoding is invalid per RFC 7541.
    #[error("invalid integer encoding: {0}")]
    InvalidInteger(String),

    /// The table index is out of range.
    #[error("invalid table index: {0}")]
    InvalidTableIndex(String),

    /// The string literal encoding is invalid.
    #[error("invalid string literal: {0}")]
    InvalidStringLiteral(String),

    /// The dynamic table size update exceeds the allowed maximum.
    #[error("table size overflow: {0}")]
    TableSizeOverflow(String),

    /// The Huffman encoding is invalid.
    #[error("invalid Huffman encoding: {0}")]
    InvalidHuffman(String),

    /// The header list exceeds the maximum allowed size.
    #[error("header list too large: {0}")]
    HeaderListTooLarge(String),
}

/// Errors returned by the HTTP/2 implementation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum H2Error {
    /// A connection-level error that requires the entire connection to be
    /// closed.
    #[error("connection error ({origin}): {reason}{detail}", origin = if *.remote { "remote" } else { "local" }, detail = message.map(|m| format!(": {m}")).unwrap_or_default())]
    ConnectionError {
        /// The error code indicating the reason for the connection error.
        reason: Reason,
        /// Whether the error originated from the remote peer.
        remote: bool,
        /// Optional descriptive message for diagnostics.
        message: Option<&'static str>,
    },

    /// A stream-level error that affects only the specified stream.
    #[error("stream error on stream {stream_id} ({origin}): {reason}", origin = if *.remote { "remote" } else { "local" })]
    StreamError {
        /// The stream ID that encountered the error.
        stream_id: u32,
        /// The error code indicating the reason for the stream reset.
        reason: Reason,
        /// Whether the error originated from the remote peer.
        remote: bool,
    },

    /// A GOAWAY frame was received from the remote peer.
    #[error("received GOAWAY: last_stream_id={last_stream_id}, reason={reason}")]
    GoAway {
        /// The last stream ID the peer will process.
        last_stream_id: u32,
        /// The error code indicating the reason for the GOAWAY.
        reason: Reason,
        /// Optional debug data from the GOAWAY frame.
        debug_data: Bytes,
    },

    /// An underlying I/O error from the transport layer.
    #[error("I/O error: {0}")]
    Io(std::sync::Arc<std::io::Error>),

    /// An HPACK header compression or decompression error.
    #[error("HPACK error: {0}")]
    Hpack(String),

    /// A frame could not be decoded due to invalid structure.
    #[error("invalid frame: {0}")]
    InvalidFrame(String),

    /// A protocol-level error with a descriptive message.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// A frame decoding error.
    #[error("frame decode error: {0}")]
    Frame(#[from] FrameError),

    /// An HPACK decoding error.
    #[error("HPACK decode error: {0}")]
    HpackDecode(#[from] HpackError),
}

impl From<std::io::Error> for H2Error {
    fn from(e: std::io::Error) -> Self {
        H2Error::Io(std::sync::Arc::new(e))
    }
}

impl H2Error {
    /// Create a locally-detected connection-level error with the given reason
    /// code.
    pub fn connection(reason: Reason) -> Self {
        H2Error::ConnectionError {
            reason,
            remote: false,
            message: None,
        }
    }

    /// Create a locally-detected connection-level error with a descriptive
    /// message.
    pub fn connection_msg(reason: Reason, message: &'static str) -> Self {
        H2Error::ConnectionError {
            reason,
            remote: false,
            message: Some(message),
        }
    }

    /// Create a connection-level error originating from the remote peer.
    pub fn connection_remote(reason: Reason) -> Self {
        H2Error::ConnectionError {
            reason,
            remote: true,
            message: None,
        }
    }

    /// Create a locally-detected stream-level error for the given stream ID and
    /// reason code.
    pub fn stream(stream_id: u32, reason: Reason) -> Self {
        H2Error::StreamError {
            stream_id,
            reason,
            remote: false,
        }
    }

    /// Create a stream-level error originating from the remote peer (RST_STREAM
    /// received).
    pub fn stream_remote(stream_id: u32, reason: Reason) -> Self {
        H2Error::StreamError {
            stream_id,
            reason,
            remote: true,
        }
    }

    /// Create a GOAWAY error from a received GOAWAY frame.
    pub fn go_away(last_stream_id: u32, reason: Reason, debug_data: Bytes) -> Self {
        H2Error::GoAway {
            last_stream_id,
            reason,
            debug_data,
        }
    }

    /// Extract the HTTP/2 reason code, if this is a connection, stream, or
    /// GOAWAY error.
    pub fn reason(&self) -> Option<Reason> {
        match self {
            H2Error::ConnectionError { reason, .. } => Some(*reason),
            H2Error::StreamError { reason, .. } => Some(*reason),
            H2Error::GoAway { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    /// Whether this is an I/O error.
    pub fn is_io(&self) -> bool {
        matches!(self, H2Error::Io(_))
    }

    /// Whether this is a stream-level reset error.
    pub fn is_reset(&self) -> bool {
        matches!(self, H2Error::StreamError { .. })
    }

    /// Whether this is a connection-level error.
    pub fn is_connection(&self) -> bool {
        matches!(self, H2Error::ConnectionError { .. })
    }

    /// Whether this is a GOAWAY error.
    pub fn is_go_away(&self) -> bool {
        matches!(self, H2Error::GoAway { .. })
    }

    /// Whether the error originated from the remote peer.
    ///
    /// GOAWAY errors are always considered remote since they are received
    /// from the peer. Connection and stream errors carry an explicit flag.
    pub fn is_remote(&self) -> bool {
        match self {
            H2Error::ConnectionError { remote, .. } => *remote,
            H2Error::StreamError { remote, .. } => *remote,
            H2Error::GoAway { .. } => true,
            _ => false,
        }
    }

    /// Whether this is a library-internal error (protocol violation,
    /// invalid frame, or HPACK error detected locally).
    pub fn is_library(&self) -> bool {
        matches!(
            self,
            H2Error::Protocol(_)
                | H2Error::InvalidFrame(_)
                | H2Error::Hpack(_)
                | H2Error::HpackDecode(_)
                | H2Error::Frame(_)
        )
    }

    /// The inner I/O error, if this is an `Io` variant.
    pub fn get_io(&self) -> Option<&std::io::Error> {
        match self {
            H2Error::Io(e) => Some(e),
            _ => None,
        }
    }

    /// Consume self and return the inner I/O error, if this is an `Io`
    /// variant.
    pub fn into_io(self) -> Option<std::io::Error> {
        match self {
            H2Error::Io(e) => std::sync::Arc::try_unwrap(e).ok(),
            _ => None,
        }
    }

    /// Extract the stream ID, if this is a stream-level error.
    pub fn stream_id(&self) -> Option<u32> {
        match self {
            H2Error::StreamError { stream_id, .. } => Some(*stream_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reason_known_roundtrip() {
        for code in 0x0..=0xd {
            let reason = Reason::from_u32(code);
            assert_eq!(reason.to_u32(), code);
            assert!(!matches!(reason, Reason::Unknown(_)));
        }
    }

    #[test]
    fn test_reason_unknown_preserved() {
        let reason = Reason::from_u32(0xFF);
        assert_eq!(reason, Reason::Unknown(0xFF));
        assert_eq!(reason.to_u32(), 0xFF);
    }

    #[test]
    fn test_reason_unknown_display() {
        let reason = Reason::Unknown(0x42);
        assert!(reason.to_string().contains("0x42"));
    }

    #[test]
    fn test_h2error_reason_connection() {
        let err = H2Error::connection(Reason::ProtocolError);
        assert_eq!(err.reason(), Some(Reason::ProtocolError));
    }

    #[test]
    fn test_h2error_reason_stream() {
        let err = H2Error::stream(5, Reason::Cancel);
        assert_eq!(err.reason(), Some(Reason::Cancel));
    }

    #[test]
    fn test_h2error_reason_none_for_io() {
        let err = H2Error::from(std::io::Error::other("test"));
        assert_eq!(err.reason(), None);
    }

    #[test]
    fn test_h2error_is_io() {
        let io_err = H2Error::from(std::io::Error::other("test"));
        assert!(io_err.is_io());
        assert!(!H2Error::connection(Reason::NoError).is_io());
    }

    #[test]
    fn test_h2error_is_reset() {
        let stream_err = H2Error::stream(3, Reason::Cancel);
        assert!(stream_err.is_reset());
        assert!(!H2Error::connection(Reason::NoError).is_reset());
    }

    #[test]
    fn test_h2error_is_connection() {
        let conn_err = H2Error::connection(Reason::ProtocolError);
        assert!(conn_err.is_connection());
        assert!(!H2Error::stream(1, Reason::Cancel).is_connection());
    }

    #[test]
    fn test_h2error_stream_id() {
        let stream_err = H2Error::stream(7, Reason::Cancel);
        assert_eq!(stream_err.stream_id(), Some(7));
        assert_eq!(H2Error::connection(Reason::NoError).stream_id(), None);
    }

    #[test]
    fn test_h2error_is_go_away() {
        let ga = H2Error::go_away(3, Reason::NoError, Bytes::new());
        assert!(ga.is_go_away());
        assert!(!H2Error::connection(Reason::NoError).is_go_away());
        assert!(!H2Error::stream(1, Reason::Cancel).is_go_away());
    }

    #[test]
    fn test_h2error_go_away_reason() {
        let ga = H2Error::go_away(5, Reason::InternalError, Bytes::from_static(b"oops"));
        assert_eq!(ga.reason(), Some(Reason::InternalError));
    }

    #[test]
    fn test_h2error_is_remote() {
        // Local errors
        assert!(!H2Error::connection(Reason::ProtocolError).is_remote());
        assert!(!H2Error::stream(1, Reason::Cancel).is_remote());

        // Remote errors
        assert!(H2Error::connection_remote(Reason::ProtocolError).is_remote());
        assert!(H2Error::stream_remote(1, Reason::Cancel).is_remote());

        // GoAway is always remote
        assert!(H2Error::go_away(0, Reason::NoError, Bytes::new()).is_remote());

        // Non-connection/stream errors
        assert!(!H2Error::Protocol("test".into()).is_remote());
        assert!(!H2Error::from(std::io::Error::other("test")).is_remote());
    }

    #[test]
    fn test_h2error_is_library() {
        assert!(H2Error::Protocol("test".into()).is_library());
        assert!(H2Error::InvalidFrame("test".into()).is_library());
        assert!(H2Error::Hpack("test".into()).is_library());
        assert!(H2Error::Frame(FrameError::InvalidFrameSize("test".into())).is_library());
        assert!(H2Error::HpackDecode(HpackError::InvalidInteger("test".into())).is_library());

        // Not library errors
        assert!(!H2Error::connection(Reason::ProtocolError).is_library());
        assert!(!H2Error::stream(1, Reason::Cancel).is_library());
        assert!(!H2Error::from(std::io::Error::other("test")).is_library());
        assert!(!H2Error::go_away(0, Reason::NoError, Bytes::new()).is_library());
    }

    #[test]
    fn test_h2error_get_io() {
        let io_err = H2Error::from(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "broken",
        ));
        assert!(io_err.get_io().is_some());
        assert_eq!(
            io_err.get_io().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );

        assert!(H2Error::connection(Reason::NoError).get_io().is_none());
    }

    #[test]
    fn test_h2error_into_io() {
        let io_err = H2Error::from(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "broken",
        ));
        let inner = io_err.into_io().unwrap();
        assert_eq!(inner.kind(), std::io::ErrorKind::BrokenPipe);

        assert!(H2Error::connection(Reason::NoError).into_io().is_none());
    }

    #[test]
    fn test_h2error_remote_constructors() {
        let local_conn = H2Error::connection(Reason::ProtocolError);
        let remote_conn = H2Error::connection_remote(Reason::ProtocolError);
        assert!(local_conn.is_connection());
        assert!(remote_conn.is_connection());
        assert!(!local_conn.is_remote());
        assert!(remote_conn.is_remote());

        let local_stream = H2Error::stream(1, Reason::Cancel);
        let remote_stream = H2Error::stream_remote(1, Reason::Cancel);
        assert!(local_stream.is_reset());
        assert!(remote_stream.is_reset());
        assert!(!local_stream.is_remote());
        assert!(remote_stream.is_remote());
    }

    #[test]
    fn test_h2error_display_formats() {
        let local = H2Error::connection(Reason::ProtocolError);
        assert!(local.to_string().contains("local"));

        let remote = H2Error::connection_remote(Reason::ProtocolError);
        assert!(remote.to_string().contains("remote"));

        let ga = H2Error::go_away(7, Reason::NoError, Bytes::new());
        assert!(ga.to_string().contains("GOAWAY"));
        assert!(ga.to_string().contains("last_stream_id=7"));
    }
}
