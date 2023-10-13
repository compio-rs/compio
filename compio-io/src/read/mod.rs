use compio_buf::{BufResult, IoBufMut, IoVectoredBufMut, SetBufInit};

mod buf;

pub use buf::*;
/// AsyncRead
///
/// Async read with a ownership of a buffer
pub trait AsyncRead {
    /// Read some bytes from this source into the buffer, which implements
    /// [`IoBufMut`], and return a [`BufResult`], consisting of the buffer and a
    /// [`usize`] indicating how many bytes were read.
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B>;

    /// Like `read`, except that it reads into a type implements
    /// [`IoVectoredBufMut`].
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V>
    where
        V::Item: IoBufMut + SetBufInit;
}

impl<A: AsyncRead + ?Sized> AsyncRead for &mut A {
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).read(buf).await
    }

    async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T>
    where
        T::Item: IoBufMut + SetBufInit,
    {
        (**self).read_vectored(buf).await
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
