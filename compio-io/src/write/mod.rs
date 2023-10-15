use std::io::Cursor;

use compio_buf::{BufResult, IntoInner, IoBuf, IoVectoredBuf};

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
    ///
    /// The default implementation will try to write from the buffers in order
    /// as if they're concatenated. It will stop whenever the writer returns
    /// an error, `Ok(0)`, or a length less than the length of the buf passed
    /// in, meaning it's possible that not all contents are written. If
    /// guaranteed full write is desired, it is recommended to use
    /// [`AsyncWriteExt::write_all_vectored`] instead.
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let mut iter = match buf.owned_iter() {
            Ok(iter) => iter,
            Err(buf) => return BufResult(Ok(0), buf),
        };
        let mut total = 0;

        loop {
            if iter.buf_len() == 0 {
                continue;
            }
            match self.write(iter).await {
                BufResult(Ok(n), ret) => {
                    iter = ret;
                    if n == 0 || n < iter.buf_len() {
                        return BufResult(Ok(total), iter.into_inner());
                    }
                    total += n;
                }
                BufResult(Err(e), ret) => return BufResult(Err(e), ret.into_inner()),
            }

            match iter.next() {
                Ok(next) => iter = next,
                Err(buf) => return BufResult(Ok(total), buf),
            }
        }
    }

    async fn flush(&mut self) -> IoResult<()>;

    async fn shutdown(&mut self) -> IoResult<()>;
}

macro_rules! impl_write {
    (@ptr $($ty:ty),*) => {
        $(
            impl<A: AsyncWrite + ?Sized> AsyncWrite for $ty {
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
        )*
    };
}

impl_write!(@ptr &mut A, Box<A>);

impl<A: AsMut<[u8]>> AsyncWrite for Cursor<A> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let pos = self.position();
        let inner = self.get_mut().as_mut();
        let mut new = Cursor::new(inner);
        new.set_position(pos);
        match std::io::Write::write(&mut new, buf.as_slice()) {
            Ok(res) => {
                self.set_position(pos + res as u64);
                BufResult(Ok(res), buf)
            }
            Err(e) => BufResult(Err(e), buf),
        }
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
        let len = buf.as_dyn_bufs().map(|b| b.buf_len()).sum();
        self.reserve(len - self.len());
        for buf in buf.as_dyn_bufs() {
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

macro_rules! impl_write_at {
    (@ptr $($ty:ty),*) => {
        $(
            impl<A: AsyncWriteAt + ?Sized> AsyncWriteAt for $ty {
                async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T> {
                    (**self).write_at(buf, pos).await
                }
            }
        )*
    };
    (@slice $($(const $len:ident =>)? $ty:ty),*) => {
        $(
            impl<$(const $len: usize)?> AsyncWriteAt for $ty {
                async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T> {
                    let slice = buf.as_slice();
                    let n = slice.len().min(self.len() - pos);
                    self[pos..pos + n].copy_from_slice(&slice[..n]);
                    BufResult(Ok(n), buf)
                }
            }
        )*
    }
}

impl_write_at!(@ptr &mut A, Box<A>);
impl_write_at!(@slice [u8], const LEN => [u8; LEN]);

impl AsyncWriteAt for Vec<u8> {
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: usize) -> BufResult<usize, T> {
        let slice = buf.as_slice();
        let n = slice.len().min(self.len() - pos);
        self[pos..pos + n].copy_from_slice(&slice[..n]);
        if n < slice.len() {
            self.extend_from_slice(&slice[n..]);
        }
        BufResult(Ok(n), buf)
    }
}
