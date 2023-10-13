use std::io::Result as IoResult;


use compio_buf::{buf_try, BufResult, IoBufMut, IoVectoredBufMut, SetBufInit};

use crate::AsyncRead;
/// # AsyncBufRead
///
/// Async read with buffered content.
///
/// ## Caution
///
/// Due to the pass-by-ownership nature of completion-based IO, the buffer is
/// passed to the inner reader when `fill_buf` is called. If the future returned
/// by `fill_buf` is dropped before inner `read` is completed, `BufReader` will
/// not be able to retrieve the buffer, causing panic.
pub trait AsyncBufRead: AsyncRead {
    /// Try fill the internal buffer with data
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]>;

    /// Mark how much data is read
    fn consume(&mut self, amount: usize);
}

impl<A: AsyncBufRead + ?Sized> AsyncBufRead for &mut A {
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]> {
        (**self).fill_buf().await
    }

    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}
