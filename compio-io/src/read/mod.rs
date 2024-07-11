#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::{io, io::Cursor, ops::DerefMut, rc::Rc, sync::Arc};

use compio_buf::{buf_try, t_alloc, BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBufMut};

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
    /// [`SetBufInit::set_buf_init`] after reading, and no further update should
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

impl<A: AsyncRead + ?Sized> AsyncRead for &mut A {
    #[inline(always)]
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).read(buf).await
    }

    #[inline(always)]
    async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).read_vectored(buf).await
    }
}

impl<R: AsyncRead + ?Sized, #[cfg(feature = "allocator_api")] A: Allocator> AsyncRead
    for t_alloc!(Box, R, A)
{
    #[inline(always)]
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).read(buf).await
    }

    #[inline(always)]
    async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        (**self).read_vectored(buf).await
    }
}

impl AsyncRead for &[u8] {
    #[inline]
    async fn read<T: IoBufMut>(&mut self, mut buf: T) -> BufResult<usize, T> {
        let len = slice_to_buf(self, &mut buf);
        *self = &self[len..];
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

/// # AsyncReadBufferPool
///
/// Async read with buffer pool
pub trait AsyncReadBufferPool {
    /// Filled buffer type
    type Buffer<'a>: DerefMut<Target = [u8]>;

    /// Buffer pool type
    type BufferPool;

    /// Read some bytes from this source with [`BufferPool`] and return
    /// a [`BorrowedBuffer`].
    ///
    /// If `len` == 0, will use [`BufferPool`] inner buffer size as the max len,
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    async fn read_buffer_pool<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>>;
}

/// # AsyncReadAtBufferPool
///
/// Async read with buffer pool and position
pub trait AsyncReadAtBufferPool {
    /// Buffer pool type
    type BufferPool;

    /// Filled buffer type
    type Buffer<'a>: DerefMut<Target = [u8]>;

    /// Read some bytes from this source at position with [`BufferPool`] and
    /// return a [`BorrowedBuffer`].
    ///
    /// If `len` == 0, will use [`BufferPool`] inner buffer size as the max len,
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    async fn read_at_buffer_pool<'a>(
        &self,
        buffer_pool: &'a Self::BufferPool,
        pos: u64,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>>;
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

    (@ptra $($ty:ident),*) => {
        $(
            #[cfg(feature = "allocator_api")]
            impl<R: AsyncReadAt + ?Sized, A: Allocator> AsyncReadAt for $ty<R, A> {
                async fn read_at<T: IoBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
                    (**self).read_at(buf, pos).await
                }

                async fn read_vectored_at<T: IoVectoredBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
                    (**self).read_vectored_at(buf, pos).await
                }
            }
            #[cfg(not(feature = "allocator_api"))]
            impl_read_at!(@ptr $ty<A>);
        )*
    };

    (@slice $($(const $len:ident =>)? $ty:ty), *) => {
        $(
            impl<$(const $len: usize)?> AsyncReadAt for $ty {
                async fn read_at<T: IoBufMut>(&self, mut buf: T, pos: u64) -> BufResult<usize, T> {
                    let pos = pos.min(self.len() as u64);
                    let len = slice_to_buf(&self[pos as usize..], &mut buf);
                    BufResult(Ok(len), buf)
                }
            }
        )*
    }
}

impl_read_at!(@ptr &A, &mut A);
impl_read_at!(@ptra Box, Rc, Arc);
impl_read_at!(@slice [u8], const LEN => [u8; LEN]);

impl<#[cfg(feature = "allocator_api")] A: Allocator> AsyncReadAt for t_alloc!(Vec, u8, A) {
    async fn read_at<T: IoBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
        self.as_slice().read_at(buf, pos).await
    }

    async fn read_vectored_at<T: IoVectoredBufMut>(&self, buf: T, pos: u64) -> BufResult<usize, T> {
        self.as_slice().read_vectored_at(buf, pos).await
    }
}

impl<A: AsyncReadAt> AsyncRead for Cursor<A> {
    #[inline]
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        let pos = self.position();
        let (n, buf) = buf_try!(self.get_ref().read_at(buf, pos).await);
        self.set_position(pos + n as u64);
        BufResult(Ok(n), buf)
    }

    #[inline]
    async fn read_vectored<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        let pos = self.position();
        let (n, buf) = buf_try!(self.get_ref().read_vectored_at(buf, pos).await);
        self.set_position(pos + n as u64);
        BufResult(Ok(n), buf)
    }
}
