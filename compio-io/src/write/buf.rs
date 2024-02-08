use std::future::ready;

use compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoVectoredBuf};

use crate::{
    buffer::Buffer,
    util::{slice_to_buf, DEFAULT_BUF_SIZE},
    AsyncWrite, IoResult,
};

/// Wraps a writer and buffers its output.
///
/// It can be excessively inefficient to work directly with something that
/// implements [`AsyncWrite`].  A `BufWriter<W>` keeps an in-memory buffer of
/// data and writes it to an underlying writer in large, infrequent batches.
//
/// `BufWriter<W>` can improve the speed of programs that make *small* and
/// *repeated* write calls to the same file or network socket. It does not
/// help when writing very large amounts at once, or writing just one or a few
/// times. It also provides no advantage when writing to a destination that is
/// in memory, like a `Vec<u8>`.
///
/// Dropping `BufWriter<W>` also discards any bytes left in the buffer, so it is
/// critical to call [`flush`] before `BufWriter<W>` is dropped. Calling
/// [`flush`] ensures that the buffer is empty and thus no data is lost.
///
/// [`flush`]: AsyncWrite::flush

#[derive(Debug)]
pub struct BufWriter<W> {
    writer: W,
    buf: Buffer,
}

impl<W> BufWriter<W> {
    /// Creates a new `BufWriter` with a default buffer capacity. The default is
    /// currently 8 KB, but may change in the future.
    pub fn new(writer: W) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, writer)
    }

    /// Creates a new `BufWriter` with the specified buffer capacity.
    pub fn with_capacity(cap: usize, writer: W) -> Self {
        Self {
            writer,
            buf: Buffer::with_capacity(cap),
        }
    }
}

impl<W: AsyncWrite> BufWriter<W> {
    async fn flush_if_needed(&mut self) -> IoResult<()> {
        if self.buf.need_flush() {
            self.flush().await?;
        }
        Ok(())
    }
}

impl<W: AsyncWrite> AsyncWrite for BufWriter<W> {
    async fn write<T: IoBuf>(&mut self, mut buf: T) -> BufResult<usize, T> {
        // The previous flush may error because disk full. We need to make the buffer
        // all-done before writing new data to it.
        (_, buf) = buf_try!(self.flush_if_needed().await, buf);

        let written = self
            .buf
            .with_sync(|w| {
                let len = w.buf_len();
                let mut w = w.slice(len..);
                let written = slice_to_buf(buf.as_slice(), &mut w);
                BufResult(Ok(written), w.into_inner())
            })
            .expect("Closure always return Ok");

        (_, buf) = buf_try!(self.flush_if_needed().await, buf);

        BufResult(Ok(written), buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, mut buf: T) -> BufResult<usize, T> {
        (_, buf) = buf_try!(self.flush_if_needed().await, buf);

        let written = self
            .buf
            .with(|mut w| {
                let mut written = 0;
                for buf in buf.as_dyn_bufs() {
                    let len = w.buf_len();
                    let mut slice = w.slice(len..);
                    written += slice_to_buf(buf.as_slice(), &mut slice);
                    w = slice.into_inner();

                    if w.buf_len() == w.buf_capacity() {
                        break;
                    }
                }
                ready(BufResult(Ok(written), w))
            })
            .await
            .expect("Closure always return Ok");

        (_, buf) = buf_try!(self.flush_if_needed().await, buf);

        BufResult(Ok(written), buf)
    }

    async fn flush(&mut self) -> IoResult<()> {
        let Self { writer, buf } = self;

        buf.flush_to(writer).await?;

        Ok(())
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        self.flush().await?;
        self.writer.shutdown().await
    }
}

impl<W> IntoInner for BufWriter<W> {
    type Inner = W;

    fn into_inner(self) -> Self::Inner {
        self.writer
    }
}
