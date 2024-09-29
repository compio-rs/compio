#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::io::Cursor;

use compio_buf::{BufResult, IntoInner, IoBuf, IoVectoredBuf, buf_try, t_alloc};

use crate::IoResult;

mod buf;
#[macro_use]
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
    ///
    /// The default implementation will try to write from the buffers in order
    /// as if they're concatenated. It will stop whenever the writer returns
    /// an error, `Ok(0)`, or a length less than the length of the buf passed
    /// in, meaning it's possible that not all contents are written. If
    /// guaranteed full write is desired, it is recommended to use
    /// [`AsyncWriteExt::write_vectored_all`] instead.
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        loop_write_vectored!(buf, total: usize, n, iter, loop self.write(iter),
            break if n == 0 || n < iter.buf_len() {
                Some(Ok(total))
            } else {
                None
            }
        )
    }

    /// Attempts to flush the object, ensuring that any buffered data reach
    /// their destination.
    async fn flush(&mut self) -> IoResult<()>;

    /// Initiates or attempts to shut down this writer, returning success when
    /// the I/O connection has completely shut down.
    async fn shutdown(&mut self) -> IoResult<()>;
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

impl<W: AsyncWrite + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator> AsyncWrite
    for t_alloc!(Box, W, A)
{
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

/// Write is implemented for `Vec<u8>` by appending to the vector. The vector
/// will grow as needed.
impl AsyncWrite for Vec<u8> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.extend_from_slice(buf.as_slice());
        BufResult(Ok(buf.buf_len()), buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let len = buf.iter_buf().map(|b| b.buf_len()).sum();
        self.reserve(len - self.len());
        for buf in buf.iter_buf() {
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
    /// Like [`AsyncWrite::write`], except that it writes at a specified
    /// position.
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T>;

    /// Like [`AsyncWrite::write_vectored`], except that it writes at a
    /// specified position.
    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        loop_write_vectored!(buf, total: u64, n, iter, loop self.write_at(iter, pos + total),
            break if n == 0 || n < iter.buf_len() {
                Some(Ok(total as usize))
            } else {
                None
            }
        )
    }
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAt for &mut A {
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
        (**self).write_at(buf, pos).await
    }

    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        (**self).write_vectored_at(buf, pos).await
    }
}

impl<W: AsyncWriteAt + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator> AsyncWriteAt
    for t_alloc!(Box, W, A)
{
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
        (**self).write_at(buf, pos).await
    }

    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        (**self).write_vectored_at(buf, pos).await
    }
}

macro_rules! impl_write_at {
    ($($(const $len:ident =>)? $ty:ty),*) => {
        $(
            impl<$(const $len: usize)?> AsyncWriteAt for $ty {
                async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
                    let pos = (pos as usize).min(self.len());
                    let slice = buf.as_slice();
                    let n = slice.len().min(self.len() - pos);
                    self[pos..pos + n].copy_from_slice(&slice[..n]);
                    BufResult(Ok(n), buf)
                }
            }
        )*
    }
}

impl_write_at!([u8], const LEN => [u8; LEN]);

/// This implementation aligns the behavior of files. If `pos` is larger than
/// the vector length, the vectored will be extended, and the extended area will
/// be filled with 0.
impl<#[cfg(feature = "allocator_api")] A: Allocator> AsyncWriteAt for t_alloc!(Vec, u8, A) {
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
        let pos = pos as usize;
        let slice = buf.as_slice();
        if pos <= self.len() {
            let n = slice.len().min(self.len() - pos);
            if n < slice.len() {
                self.reserve(slice.len() - n);
                self[pos..pos + n].copy_from_slice(&slice[..n]);
                self.extend_from_slice(&slice[n..]);
            } else {
                self[pos..pos + n].copy_from_slice(slice);
            }
            BufResult(Ok(n), buf)
        } else {
            self.reserve(pos - self.len() + slice.len());
            self.resize(pos, 0);
            self.extend_from_slice(slice);
            BufResult(Ok(slice.len()), buf)
        }
    }
}

impl<A: AsyncWriteAt> AsyncWrite for Cursor<A> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let pos = self.position();
        let (n, buf) = buf_try!(self.get_mut().write_at(buf, pos).await);
        self.set_position(pos + n as u64);
        BufResult(Ok(n), buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let pos = self.position();
        let (n, buf) = buf_try!(self.get_mut().write_vectored_at(buf, pos).await);
        self.set_position(pos + n as u64);
        BufResult(Ok(n), buf)
    }

    async fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        Ok(())
    }
}
