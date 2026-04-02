use futures_util::Stream;

use crate::{AsyncReadManaged, AsyncReadManagedAt, IoResult};

/// # AsyncReadMulti
///
/// Async read with buffer pool and returns a stream of managed buffers.
pub trait AsyncReadMulti: AsyncReadManaged {
    /// Read some bytes from this source and return a stream of
    /// [`AsyncReadManaged::Buffer`].
    ///
    /// # Implementation Note
    ///
    /// - If `len` == 0, implementation should use buffer's size as `len`
    /// - if `len` > 0, `min(len, buffer_size)` will be the max number of bytes
    ///   to be read.
    fn read_multi(&mut self, len: usize) -> impl Stream<Item = IoResult<Self::Buffer>>;
}

/// # AsyncReadMultiAt
///
/// Async read with buffer pool and position, returns a stream of managed
/// buffers.
pub trait AsyncReadMultiAt: AsyncReadManagedAt {
    /// Read some bytes from this source at position and return a stream of
    /// [`AsyncReadManagedAt::Buffer`].
    ///
    /// # Implementation Note
    ///
    /// - If `len` == 0, implementation should use buffer's size as `len`
    /// - if `len` > 0, `min(len, buffer_size)` will be the max number of bytes
    ///   to be read.
    fn read_multi_at(&self, len: usize, pos: u64) -> impl Stream<Item = IoResult<Self::Buffer>>;
}
