use std::io::Cursor;

use compio_buf::IoBuf;

use crate::IoResult;

/// # AsyncReadManaged
///
/// Async read with buffer pool
pub trait AsyncReadManaged {
    /// Filled buffer type
    type Buffer: IoBuf;

    /// Read some bytes from this source and return a [`Self::Buffer`].
    ///
    /// # Implementation Note
    ///
    /// - If `len` == 0, implementation should use buffer's size as `len`
    /// - if `len` > 0, `min(len, buffer_size)` will be the max number of bytes
    ///   to be read.
    async fn read_managed(&mut self, len: usize) -> IoResult<Self::Buffer>;
}

/// # AsyncReadAtManaged
///
/// Async read with buffer pool and position
pub trait AsyncReadManagedAt {
    /// Filled buffer type
    type Buffer: IoBuf;

    /// Read some bytes from this source at position and return a
    /// [`Self::Buffer`].
    ///
    /// # Implementation Note
    ///
    /// - If `len` == 0, implementation should use buffer's size as `len`
    /// - if `len` > 0, `min(len, buffer_size)` will be the max number of bytes
    ///   to be read.
    async fn read_managed_at(&self, len: usize, pos: u64) -> IoResult<Self::Buffer>;
}

impl<A: AsyncReadManagedAt> AsyncReadManaged for Cursor<A> {
    type Buffer = A::Buffer;

    async fn read_managed(&mut self, len: usize) -> IoResult<Self::Buffer> {
        let pos = self.position();
        let buf = self.get_ref().read_managed_at(len, pos).await?;
        self.set_position(pos + buf.buf_len() as u64);
        Ok(buf)
    }
}
