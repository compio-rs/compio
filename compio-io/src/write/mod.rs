use compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoVectoredBuf};

use crate::IoResult;

mod buf;
mod ext;

pub use buf::*;
pub use ext::*;

/// # AsyncWrite
///
/// Async write with a ownership of a buffer
pub trait AsyncWrite {
    /// Write some bytes from the buffer into this source and return a
    /// [`BufResult`], consisting of the buffer and a [`usize`] indicating how
    /// many bytes were written.
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T>;

    /// Like `write`, except that it write bytes from a buffer implements
    /// [`IoVectoredBuf`] into the source.
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T>;

    async fn flush(&mut self) -> IoResult<()>;

    async fn shutdown(&mut self) -> IoResult<()>;

    /// Write the entire contents of a buffer into this writer.
    async fn write_all<T: IoBuf>(&mut self, mut buffer: T) -> BufResult<usize, T> {
        let buf_len = buffer.buf_len();
        let mut total_written = 0;
        while total_written < buf_len {
            let written;
            (written, buffer) =
                buf_try!(self.write(buffer.slice(total_written..)).await.into_inner());
            total_written += written;
        }
        BufResult(Ok(total_written), buffer)
    }
}

impl<A: AsyncWrite + ?Sized> AsyncWrite for &mut A {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).write_vectored(buf).await
    }

    async fn flush(&mut self) -> IoResult<()> {
        (**self).flush().await
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        (**self).shutdown().await
    }
}

impl<A: AsyncWrite + ?Sized> AsyncWrite for Box<A> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).write_vectored(buf).await
    }

    async fn flush(&mut self) -> IoResult<()> {
        (**self).flush().await
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        (**self).shutdown().await
    }
}

impl AsyncWrite for &mut [u8] {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let n = buf.buf_len().min(self.len());
        self[..n].copy_from_slice(&buf.as_slice()[..n]);
        BufResult(Ok(n), buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let mut written = 0;
        for buf in buf.buf_iter() {
            let len = buf.buf_len().min(self.len() - written);
            self[written..written + len].copy_from_slice(&buf.as_slice()[..len]);
            written += len;
            if written == self.len() {
                break;
            }
        }
        BufResult(Ok(written), buf)
    }

    async fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        Ok(())
    }
}

/// Write is implemented for `Vec<u8>` by appending to the vector. The vector
/// will grow as needed.
impl AsyncWrite for Vec<u8> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.extend_from_slice(buf.as_slice());
        BufResult(Ok(buf.buf_len()), buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let len = buf.buf_iter().map(|b| b.buf_len()).sum();
        self.reserve(len);
        for buf in buf.buf_iter() {
            self.extend_from_slice(buf.as_slice());
        }
        BufResult(Ok(len), buf)
    }

    async fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        Ok(())
    }
}

/// # AsyncWriteAt
///
/// Async write with a ownership of a buffer and a position
pub trait AsyncWriteAt {
    /// Like `write`, except that it writes at a specified position.
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T>;
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAt for &mut A {
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T> {
        (**self).write_at(buf, pos).await
    }
}
