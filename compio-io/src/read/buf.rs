use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBufMut, buf_try};

use crate::{AsyncRead, IoResult, buffer::Buffer, util::DEFAULT_BUF_SIZE};
/// # AsyncBufRead
///
/// Async read with buffered content.
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

    fn consume(&mut self, amount: usize) {
        (**self).consume(amount)
    }
}

/// Wraps a reader and buffers input from [`AsyncRead`]
///
/// It can be excessively inefficient to work directly with a [`AsyncRead`]
/// instance. A `BufReader<R>` performs large, infrequent reads on the
/// underlying [`AsyncRead`] and maintains an in-memory buffer of the results.
///
/// `BufReader<R>` can improve the speed of programs that make *small* and
/// *repeated* read calls to the same file or network socket. It does not
/// help when reading very large amounts at once, or reading just one or a few
/// times. It also provides no advantage when reading from a source that is
/// already in memory, like a `Vec<u8>`.
///
/// When the `BufReader<R>` is dropped, the contents of its buffer will be
/// discarded. Reading from the underlying reader after unwrapping the
/// `BufReader<R>` with [`BufReader::into_inner`] can cause data loss.
///
/// # Caution
///
/// Due to the pass-by-ownership nature of completion-based IO, the buffer is
/// passed to the inner reader when [`fill_buf`] is called. If the future
/// returned by [`fill_buf`] is dropped before inner `read` is completed,
/// `BufReader` will not be able to retrieve the buffer, causing panic on next
/// [`fill_buf`] call.
///
/// [`fill_buf`]: #method.fill_buf
#[derive(Debug)]
pub struct BufReader<R> {
    reader: R,
    buf: Buffer,
}

impl<R> BufReader<R> {
    /// Creates a new `BufReader` with a default buffer capacity. The default is
    /// currently 8 KiB, but may change in the future.
    pub fn new(reader: R) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, reader)
    }

    /// Creates a new `BufReader` with the specified buffer capacity.
    pub fn with_capacity(cap: usize, reader: R) -> Self {
        Self {
            reader,
            buf: Buffer::with_capacity(cap),
        }
    }
}

impl<R: AsyncRead> AsyncRead for BufReader<R> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        let (mut slice, buf) = buf_try!(self.fill_buf().await, buf);
        slice.read(buf).await.map_res(|res| {
            self.consume(res);
            res
        })
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        let (mut slice, buf) = buf_try!(self.fill_buf().await, buf);
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
            buf.reset()
        }

        if buf.need_fill() {
            buf.with(|b| async move {
                let len = b.buf_len();
                let b = b.slice(len..);
                reader.read(b).await.into_inner()
            })
            .await?;
        }

        Ok(buf.slice())
    }

    fn consume(&mut self, amount: usize) {
        self.buf.advance(amount);
    }
}

impl<R> IntoInner for BufReader<R> {
    type Inner = R;

    fn into_inner(self) -> Self::Inner {
        self.reader
    }
}
