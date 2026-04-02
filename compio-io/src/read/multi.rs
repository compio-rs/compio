use futures_util::Stream;

use crate::{AsyncReadManaged, AsyncReadManagedAt, IoResult};

/// # AsyncReadMulti
///
/// Async read with buffer pool and returns a stream of managed buffers.
pub trait AsyncReadMulti: AsyncReadManaged {
    /// Read some bytes from this source with [`AsyncReadManaged::BufferPool`]
    /// and return a stream of [`AsyncReadManaged::Buffer`].
    ///
    /// If `len` == 0, will use [`AsyncReadManaged::BufferPool`] inner buffer
    /// size as the max len, if `len` > 0, `min(len, inner buffer size)` will be
    /// the read max len.
    fn read_multi(&mut self, len: usize) -> impl Stream<Item = IoResult<Self::Buffer>>;
}

/// # AsyncReadMultiAt
///
/// Async read with buffer pool and position, returns a stream of managed
/// buffers.
pub trait AsyncReadMultiAt: AsyncReadManagedAt {
    /// Read some bytes from this source at position with
    /// [`AsyncReadManagedAt::BufferPool`] and return a stream of
    /// [`AsyncReadManagedAt::Buffer`].
    ///
    /// If `len` == 0, will use [`AsyncReadManagedAt::BufferPool`] inner buffer
    /// size as the max len, if `len` > 0, `min(len, inner buffer size)` will be
    /// the read max len.
    fn read_multi_at(&self, len: usize, pos: u64) -> impl Stream<Item = IoResult<Self::Buffer>>;
}
