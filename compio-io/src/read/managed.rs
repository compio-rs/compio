use std::ops::DerefMut;

use crate::IoResult;

/// # AsyncReadManaged
///
/// Async read with buffer pool
pub trait AsyncReadManaged {
    /// Buffer pool type
    type BufferPool;
    /// Filled buffer type
    type Buffer<'a>: DerefMut<Target = [u8]>;

    /// Read some bytes from this source with [`BufferPool`] and return
    /// a [`Buffer`].
    ///
    /// If `len` == 0, will use [`BufferPool`] inner buffer size as the max len,
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> IoResult<Self::Buffer<'a>>;
}

/// # AsyncReadAtManaged
///
/// Async read with buffer pool and position
pub trait AsyncReadAtManaged {
    /// Buffer pool type
    type BufferPool;
    /// Filled buffer type
    type Buffer<'a>: DerefMut<Target = [u8]>;

    /// Read some bytes from this source at position with [`BufferPool`] and
    /// return a [`Buffer`].
    ///
    /// If `len` == 0, will use [`BufferPool`] inner buffer size as the max len,
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    async fn read_managed_at<'a>(
        &self,
        buffer_pool: &'a Self::BufferPool,
        pos: u64,
        len: usize,
    ) -> IoResult<Self::Buffer<'a>>;
}
