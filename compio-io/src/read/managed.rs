use std::io::Cursor;

use compio_buf::IoBuf;

use crate::IoResult;

/// # AsyncReadManaged
///
/// Async read with buffer pool
pub trait AsyncReadManaged {
    /// Buffer pool type
    type BufferPool;
    /// Filled buffer type
    type Buffer: IoBuf;

    /// Read some bytes from this source with [`Self::BufferPool`] and return
    /// a [`Self::Buffer`].
    ///
    /// If `len` == 0, will use [`Self::BufferPool`] inner buffer size as the
    /// max len, if `len` > 0, `min(len, inner buffer size)` will be the
    /// read max len
    async fn read_managed(
        &mut self,
        buffer_pool: &Self::BufferPool,
        len: usize,
    ) -> IoResult<Self::Buffer>;
}

/// # AsyncReadAtManaged
///
/// Async read with buffer pool and position
pub trait AsyncReadManagedAt {
    /// Buffer pool type
    type BufferPool;
    /// Filled buffer type
    type Buffer: IoBuf;

    /// Read some bytes from this source at position with [`Self::BufferPool`]
    /// and return a [`Self::Buffer`].
    ///
    /// If `len` == 0, will use [`Self::BufferPool`] inner buffer size as the
    /// max len, if `len` > 0, `min(len, inner buffer size)` will be the
    /// read max len
    async fn read_managed_at(
        &self,
        pos: u64,
        buffer_pool: &Self::BufferPool,
        len: usize,
    ) -> IoResult<Self::Buffer>;
}

impl<A: AsyncReadManagedAt> AsyncReadManaged for Cursor<A> {
    type Buffer = A::Buffer;
    type BufferPool = A::BufferPool;

    async fn read_managed(
        &mut self,
        buffer_pool: &Self::BufferPool,
        len: usize,
    ) -> IoResult<Self::Buffer> {
        let pos = self.position();
        let buf = self
            .get_ref()
            .read_managed_at(pos, buffer_pool, len)
            .await?;
        self.set_position(pos + buf.buf_len() as u64);
        Ok(buf)
    }
}
