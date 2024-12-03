use std::{io::Cursor, ops::DerefMut};

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
pub trait AsyncReadManagedAt {
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
        pos: u64,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> IoResult<Self::Buffer<'a>>;
}

impl<A: AsyncReadManagedAt> AsyncReadManaged for Cursor<A> {
    type Buffer<'a> = A::Buffer<'a>;
    type BufferPool = A::BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> IoResult<Self::Buffer<'a>> {
        let pos = self.position();
        let buf = self
            .get_ref()
            .read_managed_at(pos, buffer_pool, len)
            .await?;
        self.set_position(pos + buf.len() as u64);
        Ok(buf)
    }
}
