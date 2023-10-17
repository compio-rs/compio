use std::future::ready;

use compio_buf::{buf_try, BufResult, IntoInner, IoBuf};

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

impl<W: AsyncWrite> AsyncWrite for BufWriter<W> {
    async fn write<T: IoBuf>(&mut self, mut buf: T) -> compio_buf::BufResult<usize, T> {
        let written = self
            .buf
            .with(|mut w| {
                let written = slice_to_buf(buf.as_slice(), &mut w);
                ready(BufResult(Ok(written), w))
            })
            .await
            .expect("Closure always return Ok");

        if self.buf.need_flush() {
            (_, buf) = buf_try!(self.flush().await, buf);
        }

        BufResult(Ok(written), buf)
    }

    async fn write_vectored<T: compio_buf::IoVectoredBuf>(
        &mut self,
        mut buf: T,
    ) -> compio_buf::BufResult<usize, T> {
        let written = self
            .buf
            .with(|mut w| {
                let mut written = 0;
                for buf in buf.as_dyn_bufs() {
                    written += slice_to_buf(buf.as_slice(), &mut w);

                    if w.buf_len() == w.buf_capacity() {
                        break;
                    }
                }
                ready(BufResult(Ok(written), w))
            })
            .await
            .expect("Closure always return Ok");

        if self.buf.need_flush() {
            (_, buf) = buf_try!(self.flush().await, buf);
        }

        BufResult(Ok(written), buf)
    }

    async fn flush(&mut self) -> IoResult<()> {
        let Self { writer, buf } = self;

        let len = buf.with(|w| writer.write(w)).await?;
        buf.advance(len);

        if buf.all_done() {
            buf.reset();
        }

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
