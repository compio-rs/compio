//! QUIC streams
use quiche::Shutdown;

use super::QuicResult;
use crate::quic::connection::Connection;

/// A QUIC stream id, represented as a [`u64`].
///
/// First 62 bits the actual stream id number, the second least significant bit
/// is used to identify the direction of the stream (1 for uni-directional
/// and 0 for bi-directional). The least significant bit is used to identify the
/// initiator of the stream (1 for server-initiated and 0 for client-initiated).
///
/// See [RFC 9000 Section 2.1] for more details.
///
/// [RFC 9000 Section 2.1]: https://tools.ietf.org/html/rfc9000#section-2.1
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(u64);

impl StreamId {
    const CLIENT_BI: Self = Self(0b00);
    const CLIENT_UNI: Self = Self(0b10);
    const SERVER_BI: Self = Self(0b01);
    const SERVER_UNI: Self = Self(0b11);

    #[inline]
    /// Create a new stream id with given number
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Create a new bidirectional stream id
    #[inline]
    pub const fn new_bi(id: u64, is_server: bool) -> Self {
        if is_server {
            Self::SERVER_BI.with_num(id)
        } else {
            Self::CLIENT_BI.with_num(id)
        }
    }

    /// Create a new unidirectional stream id
    #[inline]
    pub const fn new_uni(id: u64, is_server: bool) -> Self {
        if is_server {
            Self::SERVER_UNI.with_num(id)
        } else {
            Self::CLIENT_UNI.with_num(id)
        }
    }

    /// Get the underlying u64 value
    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Convert the stream id to a bidirectional stream id
    #[inline]
    pub const fn to_bi(self) -> Self {
        Self(self.0 & !0b10)
    }

    /// Convert the stream id to a unidirectional stream id
    #[inline]
    pub const fn to_uni(self) -> Self {
        Self(self.0 | 0b10)
    }

    /// Get current stream id and increment self to the next stream id
    #[inline]
    pub fn into_next(&mut self) -> Self {
        let curr = Self(self.0 + 4);
        *self = curr;
        curr
    }

    /// If the stream is client-initiated
    #[inline]
    pub fn by_client(&self) -> bool {
        self.0 & 0b1 == 0b00
    }

    /// If the stream is server-initiated
    #[inline]
    pub fn by_server(&self) -> bool {
        self.0 & 0b1 == 0b01
    }

    /// If the stream is bidirectional
    #[inline]
    pub fn is_bi(&self) -> bool {
        self.0 & 0b10 == 0b00
    }

    /// If the stream is unidirectional
    #[inline]
    pub fn is_uni(&self) -> bool {
        self.0 & 0b10 == 0b10
    }

    const fn with_num(&self, num: u64) -> Self {
        assert!(num < 2u64.pow(62));

        Self(num << 2 | self.0)
    }
}

impl std::ops::BitAnd for StreamId {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

/// Shared methods for both unidirectional and bidirectional streams
macro_rules! shared_fn {
    () => {
        pub(super) fn new(connection: Connection, id: StreamId) -> QuicResult<Self> {
            Ok(Self { connection, id })
        }

        /// Get the stream id
        pub fn id(&self) -> StreamId {
            self.id
        }

        /// If the stream is client-initiated
        pub fn by_client(&self) -> bool {
            self.id().by_client()
        }

        /// If the stream is server-initiated
        pub fn by_server(&self) -> bool {
            self.id().by_server()
        }

        /// If the stream is readable
        pub fn readable(&self) -> bool {
            self.connection
                .with(|s| s.quic.stream_readable(self.id.as_u64()))
        }

        /// If the stream has enough space to write `len` bytes.
        ///
        /// If it doesn't, the peer will be notified and will this stream will not be
        /// woke up until the it has enough space to write `len` bytes.
        ///
        /// # Error
        ///
        /// - If the stream is closed, an [`quiche::Error::InvalidStreamState`] will be
        /// returned.
        /// - If the peer has signalled that it doesn't want to receive more data, an
        ///   [`quiche::Error::StreamStopped`] error will be returned.
        ///
        /// See [`stream_writable`] for more details.
        ///
        /// [`stream_writable`]: quiche::Connection::stream_writable
        pub fn writable(&mut self, len: usize) -> QuicResult<bool> {
            self.connection
                .with(|s| s.quic.stream_writable(self.id.as_u64(), len))
                .map_err(Into::into)
        }
    };
}

/// An uni-directional QUIC stream
pub struct UniStream {
    id: StreamId,
    connection: Connection,
}

impl UniStream {
    shared_fn!();

    /// Shutdown the stream
    pub fn shutdown(&mut self, error_code: u64) -> QuicResult<()> {
        // Only shutdown locally created unidirectional stream
        if self.id().by_server() != self.connection.with(|s| s.is_server()) {
            return Ok(());
        }

        self.connection
            .get()
            .quic
            .stream_shutdown(self.id.as_u64(), Shutdown::Write, error_code)
            .map_err(Into::into)
    }
}

/// A bi-directional QUIC stream
pub struct BiStream {
    id: StreamId,
    connection: Connection,
}

impl BiStream {
    shared_fn!();

    /// Shutdown the stream
    pub fn shutdown(&self, error_code: u64, direction: Shutdown) -> QuicResult<()> {
        self.connection
            .get()
            .quic
            .stream_shutdown(self.id.as_u64(), direction, error_code)
            .map_err(Into::into)
    }
}
