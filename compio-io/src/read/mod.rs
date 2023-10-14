use std::{io::Cursor, rc::Rc, sync::Arc};

use compio_buf::{BufResult, IoBufMut, IoVectoredBufMut};

mod buf;
mod ext;

pub use buf::*;
pub use ext::*;

use crate::util::slice_to_buf;

/// AsyncRead
///
/// Async read with a ownership of a buffer
pub trait AsyncRead {
    /// Read some bytes from this source into the [`IoBufMut`] buffer and return
    /// a [`BufResult`], consisting of the buffer and a [`usize`] indicating
    /// how many bytes were read.

    /// # Caution
    ///
    /// Implementor **MUST** update the buffer init via
    /// [`SetBufInit::set_buf_init`] after reading, and no furthur update should
    /// be made by caller.
    ///
    /// [`SetBufInit::set_buf_init`]: compio_buf::SetBufInit::set_buf_init
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B>;

    /// Like `read`, except that it reads into a type implements
    /// [`IoVectoredBufMut`].
    ///
    /// # Caution
    ///
    /// Implementor **MUST** update the buffer init via
    /// [`SetBufInit::set_buf_init`] after reading.
    ///
    /// [`SetBufInit::set_buf_init`]: compio_buf::SetBufInit::set_buf_init
    }
}

macro_rules! impl_read {
    ($($ty:ty),*) => {
        $(
            impl<A: AsyncRead + ?Sized> AsyncRead for $ty {
                #[inline(always)]
                async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
                    (**self).read(buf).await
                }

                #[inline(always)]
                async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T>

                {
                    (**self).read_vectored(buf).await
                }
            }
        )*
    };
}

impl_read!(&mut A, Box<A>);

impl<A: AsRef<[u8]>> AsyncRead for Cursor<A> {
    #[inline]
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        self.get_ref().as_ref().read(buf).await
    }

    #[inline]
    async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        self.get_ref().as_ref().read_vectored(buf).await
    }
}

impl<const T: usize> AsyncRead for [u8; T] {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&self[..]).read(buf).await
    }
}

impl<const T: usize> AsyncRead for &[u8; T] {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&self[..]).read(buf).await
    }
}

impl AsyncRead for String {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.as_bytes().read(buf).await
    }
}

impl AsyncRead for &String {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.as_bytes().read(buf).await
    }
}

impl AsyncRead for &'_ str {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.as_bytes().read(buf).await
    }
}

impl AsyncRead for &[u8] {
    #[inline]
    async fn read<T: IoBufMut>(&mut self, mut buf: T) -> BufResult<usize, T> {
        let len = slice_to_buf(self, &mut buf);

        BufResult(Ok(len), buf)
    }

    async fn read_vectored<T: IoVectoredBufMut>(&mut self, mut buf: T) -> BufResult<usize, T>
    where
        T::Item: IoBufMut,
    {
        let mut this = *self; // An immutable slice to track the read position

        for buf in buf.buf_iter_mut() {
            let n = slice_to_buf(this, buf);
            this = &this[n..];
            if this.is_empty() {
                break;
            }
        }

        BufResult(Ok(self.len() - this.len()), buf)
    }
}

/// # AsyncReadAt
///
/// Async read with a ownership of a buffer and a position
pub trait AsyncReadAt {
    /// Like `read`, except that it reads at a specified position.
    async fn read_at<T: IoBufMut>(&self, buf: T, pos: usize) -> BufResult<usize, T>;
}

macro_rules! impl_read_at {
    ($($ty:ty),*) => {
        $(
            impl<A: AsyncReadAt + ?Sized> AsyncReadAt for $ty {
                async fn read_at<T: IoBufMut>(&self, buf: T, pos: usize) -> BufResult<usize, T> {
                    (**self).read_at(buf, pos).await
                }
            }
        )*
    };
}

impl_read_at!(&A, &mut A, Box<A>, Rc<A>, Arc<A>);
