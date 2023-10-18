use std::{io::Cursor, rc::Rc, sync::Arc};

use compio_buf::{buf_try, BufResult, IntoInner, IoBufMut, IoVectoredBufMut};

mod buf;
#[macro_use]
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
    ///
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
    /// The default implementation will try to read into the buffers in order,
    /// and stop whenever the reader returns an error, `Ok(0)`, or a length
    /// less than the length of the buf passed in, meaning it's possible that
    /// not all buffer space is filled. If guaranteed full read is desired,
    /// it is recommended to use [`AsyncReadExt::read_vectored_exact`]
    /// instead.
    ///
    /// # Caution
    ///
    /// Implementor **MUST** update the buffer init via
    /// [`SetBufInit::set_buf_init`] after reading.
    ///
    /// [`SetBufInit::set_buf_init`]: compio_buf::SetBufInit::set_buf_init
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        loop_read_vectored!(
            buf, len, total: usize, n, iter,
            loop self.read(iter),
            break if n == 0 || n < len {
                Some(Ok(total))
            } else {
                None
            }
        )
    }
}

macro_rules! impl_read {
    (@ptr $($ty:ty),*) => {
        $(
            impl<A: AsyncRead + ?Sized> AsyncRead for $ty {
                #[inline(always)]
                async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
                    (**self).read(buf).await
                }

                #[inline(always)]
                async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
                    (**self).read_vectored(buf).await
                }
            }
        )*
    };

    (@slice $ty:ty, for $($tt:tt)*) => {
        impl<$($tt)*> AsyncRead for $ty {
            #[inline(always)]
            async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
                (&self[..]).read(buf).await
            }
        }
    };

    (@string $($ty:ty),*) => {
        $(
            impl AsyncRead for $ty {
                #[inline(always)]
                async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
                    self.as_bytes().read(buf).await
                }
            }
        )*
    };
}

impl_read!(@ptr &mut A, Box<A>);
impl_read!(@slice [u8], for);
impl_read!(@slice [u8; LEN], for const LEN: usize);
impl_read!(@slice &[u8; LEN], for const LEN: usize);
impl_read!(@string String, &'_ str, &String);

impl<A: AsRef<[u8]>> AsyncRead for Cursor<A> {
    #[inline]
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        let len = self.position().min(self.get_ref().as_ref().len() as u64);
        let (n, buf) = buf_try!((&self.get_ref().as_ref()[(len as usize)..]).read(buf).await);
        let pos = (self.position() as usize).checked_add(n).expect("overflow");

        debug_assert!(pos <= self.get_ref().as_ref().len());

        self.set_position(pos as u64);
        BufResult(Ok(n), buf)
    }

    #[inline]
    async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        let len = self.position().min(self.get_ref().as_ref().len() as u64);
        let (n, buf) = buf_try!(
            (&self.get_ref().as_ref()[(len as usize)..])
                .read_vectored(buf)
                .await
        );
        let pos = (self.position() as usize).checked_add(n).expect("overflow");

        debug_assert!(pos <= self.get_ref().as_ref().len());

        self.set_position(pos as u64);
        BufResult(Ok(n), buf)
    }
}

impl AsyncRead for &[u8] {
    #[inline]
    async fn read<T: IoBufMut>(&mut self, mut buf: T) -> BufResult<usize, T> {
        let len = slice_to_buf(self, &mut buf);

        BufResult(Ok(len), buf)
    }

    async fn read_vectored<T: IoVectoredBufMut>(&mut self, mut buf: T) -> BufResult<usize, T> {
        let mut this = *self; // An immutable slice to track the read position

        for buf in buf.as_dyn_mut_bufs() {
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
    /// Like [`AsyncRead::read`], except that it reads at a specified position.
    async fn read_at<T: IoBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T>;

    /// Like [`AsyncRead::read_vectored`], except that it reads at a specified
    /// position.
    async fn read_vectored_at<T: IoVectoredBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
        loop_read_vectored!(
            buf, len, total: u64, n, iter,
            loop self.read_at(iter, pos + total),
            break if n == 0 || n < len {
                Some(Ok(total as usize))
            } else {
                None
            }
        )
    }
}

macro_rules! impl_read_at {
    (@ptr $($ty:ty),*) => {
        $(
            impl<A: AsyncReadAt + ?Sized> AsyncReadAt for $ty {
                async fn read_at<T: IoBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
                    (**self).read_at(buf, pos).await
                }

                async fn read_vectored_at<T: IoVectoredBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
                    (**self).read_vectored_at(buf, pos).await
                }
            }
        )*
    };

    (@slice $($(const $len:ident =>)? $ty:ty), *) => {
        $(
            impl<$(const $len: usize)?> AsyncReadAt for $ty {
                async fn read_at<T: IoBufMut>(&self, mut buf: T, pos: u64) -> BufResult<usize, T> {
                    let len = slice_to_buf(&self[pos as usize..], &mut buf);
                    BufResult(Ok(len), buf)
                }
            }
        )*
    }
}

impl_read_at!(@ptr &A, &mut A, Box<A>, Rc<A>, Arc<A>);
impl_read_at!(@slice [u8], const LEN => [u8; LEN]);
