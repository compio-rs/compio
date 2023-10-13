use std::io::Result as IoResult;

use compio_buf::{BufResult, IoBufMut, IoVectoredBufMut};

/// AsyncRead
///
/// Async read with a ownership of a buffer
pub trait AsyncRead {
    /// Read some bytes from this source into the buffer, which implements
    /// [`IoBufMut`], and return a [`BufResult`], consisting of the buffer and a
    /// [`usize`] indicating how many bytes were read.
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T>;

    /// Like `read`, except that it reads into a type implements
    /// [`IoVectoredBufMut`].
    async fn readv<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T>;
}

impl<A: AsyncRead + ?Sized> AsyncRead for &mut A {
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).read(buf).await
    }

    async fn readv<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).readv(buf).await
    }
}
/// # AsyncReadAt
///
/// Async read with a ownership of a buffer and a position
pub trait AsyncReadAt {
    /// Like `read`, except that it reads at a specified position.
    async fn read_at<T: IoBufMut>(&mut self, buf: T, pos: usize) -> BufResult<usize, T>;
}

impl<A: AsyncReadAt + ?Sized> AsyncReadAt for &mut A {
    async fn read_at<T: IoBufMut>(&mut self, buf: T, pos: usize) -> BufResult<usize, T> {
        (**self).read_at(buf, pos).await
    }
}

/// # AsyncBufRead
///
/// Async read with buffered content
pub trait AsyncBufRead: AsyncRead {
    /// Try fill the internal buffer with data
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]>;

    /// Mark how much data is read
    fn consume(&mut self, amt: usize);
}

impl<A: AsyncBufRead + ?Sized> AsyncBufRead for &mut A {
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]> {
        (**self).fill_buf().await
    }

    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}
