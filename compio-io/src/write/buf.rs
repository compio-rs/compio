use std::io::Result as IoResult;

use crate::AsyncWrite;

/// # AsyncBufWrite
///
/// Async write with buffered content
pub trait AsyncBufWrite: AsyncWrite {
    /// Try write data and get a reference to the internal buffer
    async fn flush_buf(&mut self) -> IoResult<()>;
}

impl<A: AsyncBufWrite + ?Sized> AsyncBufWrite for &mut A {
    async fn flush_buf(&mut self) -> IoResult<()> {
        (**self).flush_buf().await
    }
}
