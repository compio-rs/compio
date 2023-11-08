//! QUIC streams
use compio_runtime::event::Event;
use quiche::Shutdown;

use super::{QuicResult, SharedInner};

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

    /// Create a new bidirectional stream id
    pub const fn new_bi(num: u64, is_server: bool) -> Self {
        if is_server {
            Self::SERVER_BI.with_num(num)
        } else {
            Self::CLIENT_BI.with_num(num)
        }
    }

    /// Create a new unidirectional stream id
    pub const fn new_uni(num: u64, is_server: bool) -> Self {
        if is_server {
            Self::SERVER_UNI.with_num(num)
        } else {
            Self::CLIENT_UNI.with_num(num)
        }
    }

    /// Get the underlying u64 value
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// Convert the stream id to a bidirectional stream id
    pub const fn to_bi(self) -> Self {
        Self(self.0 & !0b10)
    }

    /// Convert the stream id to a unidirectional stream id
    pub const fn to_uni(self) -> Self {
        Self(self.0 | 0b10)
    }

    /// Get current stream id and increment self to the next stream id
    pub fn into_next(&mut self) -> Self {
        let curr = Self(self.0 + 4);
        *self = curr;
        curr
    }

    /// If the stream is client-initiated
    pub fn by_client(&self) -> bool {
        self.0 & 0b1 == 0b00
    }

    /// If the stream is server-initiated
    pub fn by_server(&self) -> bool {
        self.0 & 0b1 == 0b01
    }

    /// If the stream is bidirectional
    pub fn is_bi(&self) -> bool {
        self.0 & 0b10 == 0b00
    }

    /// If the stream is unidirectional
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
        pub(super) fn new(inner: SharedInner, id: StreamId) -> QuicResult<Self> {
            let event = compio_runtime::event::Event::new()?;
            let event_handle = event.handle()?;
            inner.with(|s| s.streams.insert(id, event_handle));
            Ok(Self { inner, event, id })
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
            self.inner
                .with(|s| s.quic.stream_readable(self.id.as_u64()))
        }

        /// If the stream has enough space to write `len` bytes
        ///
        /// See [`stream_writable`] for more details.
        ///
        /// [`stream_writable`]: quiche::Connection::stream_writable
        pub fn writable(&self, len: usize) -> QuicResult<bool> {
            self.inner
                .with(|s| s.quic.stream_writable(self.id.as_u64(), len))
                .map_err(Into::into)
        }
    };
}

/// A uni-directional QUIC stream
pub struct UniStream {
    inner: SharedInner,
    event: Event,
    id: StreamId,
}

impl UniStream {
    shared_fn!();

    /// Shutdown the stream
    pub fn shutdown(&self, error_code: u64) -> QuicResult<()> {
        // Only shutdown locally created unidirectional stream
        if self.id().by_server() != self.inner.with(|s| s.is_server) {
            return Ok(());
        }

        self.inner
            .get()
            .quic
            .stream_shutdown(self.id.as_u64(), Shutdown::Write, error_code)
            .map_err(Into::into)
    }
}

/// A bi-directional QUIC stream
pub struct BiStream {
    inner: SharedInner,
    event: Event,
    id: StreamId,
}

impl BiStream {
    shared_fn!();

    /// Shutdown the stream
    pub fn shutdown(&self, error_code: u64, direction: Shutdown) -> QuicResult<()> {
        self.inner
            .get()
            .quic
            .stream_shutdown(self.id.as_u64(), direction, error_code)
            .map_err(Into::into)
    }
}
