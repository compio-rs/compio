use std::{io, ops::Deref};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, BufferRef, RawFd, SharedFd, ToSharedFd,
    op::{BufResultExt, Read, ReadManaged, ReadMulti, ResultTakeBuffer, Write},
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncReadMulti, AsyncWrite, util::Splittable};
use futures_util::{Stream, future::Either};
#[cfg(unix)]
use {
    compio_buf::{IoVectoredBuf, IoVectoredBufMut},
    compio_driver::op::{ReadVectored, WriteVectored},
};

use crate::{Attacher, Runtime};

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

/// Providing implementations for [`AsyncRead`] and [`AsyncWrite`].
#[derive(Debug)]
pub struct AsyncFd<T: AsFd> {
    inner: Attacher<T>,
}

impl<T: AsFd> AsyncFd<T> {
    /// Create [`AsyncFd`] and attach the source to the current runtime.
    pub fn new(source: T) -> io::Result<Self> {
        Ok(Self {
            inner: Attacher::new(source)?,
        })
    }

    /// Create [`AsyncFd`] without attaching the source.
    ///
    /// # Safety
    ///
    /// * The user should handle the attachment correctly.
    /// * `T` should be an owned fd.
    pub unsafe fn new_unchecked(source: T) -> Self {
        Self {
            inner: unsafe { Attacher::new_unchecked(source) },
        }
    }
}

impl<T: AsFd + 'static> AsyncRead for AsyncFd<T> {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).read(buf).await
    }

    #[cfg(unix)]
    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        (&*self).read_vectored(buf).await
    }
}

impl<T: AsFd + 'static> AsyncRead for &AsyncFd<T> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        let fd = self.inner.to_shared_fd();
        let op = Read::new(fd, buf);
        let res = crate::submit(op).await.into_inner();
        unsafe { res.map_advanced() }
    }

    #[cfg(unix)]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        use compio_driver::op::VecBufResultExt;

        let fd = self.inner.to_shared_fd();
        let op = ReadVectored::new(fd, buf);
        let res = crate::submit(op).await.into_inner();
        unsafe { res.map_vec_advanced() }
    }
}

impl<T: AsFd + 'static> AsyncReadManaged for AsyncFd<T> {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        (&*self).read_managed(len).await
    }
}

impl<T: AsFd + 'static> AsyncReadManaged for &AsyncFd<T> {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        let runtime = Runtime::current();
        let fd = self.to_shared_fd();
        let op = ReadManaged::new(fd, &runtime.buffer_pool()?, len)?;
        let res = runtime.submit(op).await;
        unsafe { res.take_buffer() }
    }
}

impl<T: AsFd + 'static> AsyncReadMulti for AsyncFd<T> {
    fn read_multi(&mut self, len: usize) -> impl Stream<Item = io::Result<Self::Buffer>> {
        let fd = self.to_shared_fd();
        read_multi(fd, len)
    }
}

impl<T: AsFd + 'static> AsyncReadMulti for &AsyncFd<T> {
    fn read_multi(&mut self, len: usize) -> impl Stream<Item = io::Result<Self::Buffer>> {
        let fd = self.to_shared_fd();
        read_multi(fd, len)
    }
}

fn read_multi<T: AsFd + 'static>(
    fd: SharedFd<T>,
    len: usize,
) -> impl Stream<Item = io::Result<BufferRef>> {
    let runtime = Runtime::current();
    let pool = runtime.buffer_pool();
    pool.and_then(|pool| Ok((ReadMulti::new(fd, &pool, len)?, pool)))
        .map(|(op, pool)| runtime.submit_multi(op).into_managed(pool))
        .map(Either::Left)
        .unwrap_or_else(|e| Either::Right(futures_util::stream::once(std::future::ready(Err(e)))))
}

impl<T: AsFd + 'static> AsyncWrite for AsyncFd<T> {
    #[inline]
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).write(buf).await
    }

    #[cfg(unix)]
    #[inline]
    async fn write_vectored<V: IoVectoredBuf>(&mut self, buf: V) -> BufResult<usize, V> {
        (&*self).write_vectored(buf).await
    }

    #[inline]
    async fn flush(&mut self) -> io::Result<()> {
        (&*self).flush().await
    }

    #[inline]
    async fn shutdown(&mut self) -> io::Result<()> {
        (&*self).shutdown().await
    }
}

impl<T: AsFd + 'static> AsyncWrite for &AsyncFd<T> {
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        let fd = self.inner.to_shared_fd();
        let op = Write::new(fd, buf);
        crate::submit(op).await.into_inner()
    }

    #[cfg(unix)]
    async fn write_vectored<V: IoVectoredBuf>(&mut self, buf: V) -> BufResult<usize, V> {
        let fd = self.inner.to_shared_fd();
        let op = WriteVectored::new(fd, buf);
        crate::submit(op).await.into_inner()
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<T: AsFd> IntoInner for AsyncFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.inner.into_inner()
    }
}

impl<T: AsFd> AsFd for AsyncFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

impl<T: AsFd> AsRawFd for AsyncFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_fd().as_raw_fd()
    }
}

impl<T: AsFd> ToSharedFd<T> for AsyncFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.inner.to_shared_fd()
    }
}

impl<T: AsFd> Clone for AsyncFd<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: AsFd> Deref for AsyncFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: AsFd> Splittable for AsyncFd<T> {
    type ReadHalf = Self;
    type WriteHalf = Self;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (self.clone(), self)
    }
}
