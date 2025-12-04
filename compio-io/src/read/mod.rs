#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::{io::Cursor, rc::Rc, sync::Arc};

use compio_buf::{BufResult, IntoInner, IoBufMut, IoVectoredBufMut, buf_try, t_alloc};

mod buf;
#[macro_use]
mod ext;
mod managed;

pub use buf::*;
pub use ext::*;
pub use managed::*;

use crate::util::{slice_to_buf, slice_to_uninit};

/// AsyncRead
///
/// Async read with a ownership of a buffer
pub trait AsyncRead {
    /// Read some bytes from this source into the [`IoBufMut`] buffer and return
    /// a [`BufResult`], consisting of the buffer and a [`usize`] indicating
    /// how many bytes were read.
    ///
    /// # Caution
    /// - This function read data to the **beginning** of the buffer; that is,
    ///   all existing data in the buffer will be overwritten. To read data to
    ///   the end of the buffer, use [`AsyncReadExt::append`].
    /// - Implementor **MUST** update the buffer init via
    ///   [`SetBufInit::set_buf_init`] after reading, and no further update
    ///   should be made by caller.
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
        loop_read_vectored!(buf, iter, self.read(iter))
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

        for buf in buf.iter_uninit_slice() {
            let n = slice_to_uninit(this, buf);
            this = &this[n..];
            if this.is_empty() {
                break;
            }
        }

        let len = self.len() - this.len();
        *self = this;

        unsafe {
            buf.set_buf_init(len);
        }

        BufResult(Ok(len), buf)
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
        loop_read_vectored!(buf, iter, self.read_at(iter, pos))
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

                async fn read_vectored_at<T:IoVectoredBufMut>(&self, mut buf: T, pos: u64) -> BufResult<usize, T> {
                    let slice = &self[pos as usize..];
                    let mut this = slice;

                    for buf in buf.iter_uninit_slice() {
                        let n = slice_to_uninit(this, buf);
                        this = &this[n..];
                        if this.is_empty() {
                            break;
                        }
                    }

                    let len = slice.len() - this.len();
                    unsafe {
                        buf.set_buf_init(len);
                    }

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
