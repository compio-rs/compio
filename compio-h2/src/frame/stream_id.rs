use std::fmt;

/// HTTP/2 stream identifier (31-bit unsigned integer).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct StreamId(u32);

impl StreamId {
    /// Mask for the 31-bit stream identifier (high bit is reserved).
    const MASK: u32 = 0x7FFF_FFFF;
    /// The zero stream ID, used for connection-level frames.
    pub const ZERO: StreamId = StreamId(0);

    /// Create a new stream ID, masking off the reserved high bit per RFC 9113
    /// Section 4.1.
    pub fn new(id: u32) -> Self {
        StreamId(id & Self::MASK)
    }

    /// The 31-bit stream identifier value.
    pub fn value(&self) -> u32 {
        self.0
    }

    /// Whether this is the zero (connection-level) stream ID.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Client-initiated streams have odd IDs.
    pub fn is_client_initiated(&self) -> bool {
        !self.is_zero() && self.0 % 2 == 1
    }

    /// Server-initiated streams have even IDs.
    pub fn is_server_initiated(&self) -> bool {
        !self.is_zero() && self.0.is_multiple_of(2)
    }

    /// The next stream ID (increments by 2 to preserve odd/even
    /// parity).
    pub fn next_id(&self) -> Option<StreamId> {
        let next = self.0.checked_add(2)?;
        if next & Self::MASK != next {
            return None;
        }
        Some(StreamId(next))
    }
}

impl From<u32> for StreamId {
    fn from(val: u32) -> Self {
        StreamId::new(val)
    }
}

impl From<StreamId> for u32 {
    fn from(id: StreamId) -> u32 {
        id.0
    }
}

impl fmt::Debug for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamId({})", self.0)
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_id_basics() {
        let id = StreamId::new(1);
        assert!(!id.is_zero());
        assert!(id.is_client_initiated());
        assert!(!id.is_server_initiated());

        let id = StreamId::new(2);
        assert!(id.is_server_initiated());
        assert!(!id.is_client_initiated());

        assert!(StreamId::ZERO.is_zero());
    }

    #[test]
    fn test_stream_id_next() {
        let id = StreamId::new(1);
        assert_eq!(id.next_id(), Some(StreamId::new(3)));

        let id = StreamId::new(2);
        assert_eq!(id.next_id(), Some(StreamId::new(4)));
    }

    #[test]
    fn test_stream_id_mask() {
        // High bit should be masked off
        let id = StreamId::new(0x8000_0001);
        assert_eq!(id.value(), 1);
    }

    #[test]
    fn test_stream_id_overflow() {
        let id = StreamId::new(0x7FFF_FFFF);
        assert_eq!(id.next_id(), None);
    }
}
