//! QUIC streams
use super::{QuicResult, SharedInner};

pub(crate) const INITIALIZER_MASK: u64 = 0b01;
pub(crate) const SERVER_STREAM: u64 = 0b01;
pub(crate) const CLIENT_STREAM: u64 = 0b00;

/// Shared methods for both unidirectional and bidirectional streams
macro_rules! shared_fn {
    () => {
        pub(super) fn new(inner: SharedInner, id: u64) -> Self {
            Self { inner, id }
        }

        /// Get the stream id
        pub fn id(&self) -> u64 {
            self.id
        }

        /// See if the stream is a client-initiated stream
        pub fn by_client(&self) -> bool {
            self.id & INITIALIZER_MASK == CLIENT_STREAM
        }

        /// See if the stream is a server-initiated stream
        pub fn by_server(&self) -> bool {
            self.id & INITIALIZER_MASK == SERVER_STREAM
        }

        /// See if the stream is readable
        pub fn readable(&self) -> bool {
            self.inner.with(|s| s.quic.stream_readable(self.id))
        }

        /// See if the stream has enough space to write `len` bytes
        ///
        /// See [`stream_writable`] for more details.
        ///
        /// [`stream_writable`]: quiche::Connection::stream_writable
        pub fn writable(&self, len: usize) -> QuicResult<bool> {
            self.inner
                .with(|s| s.quic.stream_writable(self.id, len))
                .map_err(Into::into)
        }
    };
}

pub struct UniStream {
    inner: SharedInner,
    id: u64,
}

impl UniStream {
    shared_fn!();
}

pub struct BiStream {
    inner: SharedInner,
    id: u64,
}

impl BiStream {
    shared_fn!();
}
