use compio_buf::{buf_try, BufResult, IoBufMut, IoVectoredBufMut};

use crate::{buffer::Buffer, util::DEFAULT_BUF_SIZE, AsyncRead, IoResult};
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

pub struct BufReader<R> {
    reader: R,
    buf: Buffer,
}

impl<R> BufReader<R> {
    pub fn new(reader: R) -> Self {
        Self::with_capacity(reader, DEFAULT_BUF_SIZE)
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    pub fn with_capacity(reader: R, cap: usize) -> Self {
        Self {
            reader,
            buf: Buffer::with_capacity(cap),
        }
    }
}

impl<R: AsyncRead> AsyncRead for BufReader<R> {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let mut slice;
        (slice, buf) = buf_try!(self.fill_buf().await, buf);
        slice.read(buf).await.map_res(|res| {
            self.consume(res);
            res
        })
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, mut buf: V) -> BufResult<usize, V>
    where
        V::Item: IoBufMut,
    {
        let mut slice;
        (slice, buf) = buf_try!(self.fill_buf().await, buf);
        slice.read_vectored(buf).await.map_res(|res| {
            self.consume(res);
            res
        })
    }
}

impl<R: AsyncRead> AsyncBufRead for BufReader<R> {
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]> {
        let Self { reader, buf } = self;

        if buf.all_done() {
            buf.clear()
        }

        if buf.need_fill() {
            buf.with(|b| reader.read(b)).await?;
        }

        Ok(buf.slice())
    }

    fn consume(&mut self, amt: usize) {
        self.buf.advance(amt)
    }
}
