#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::{
    AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, RawHandle, RawSocket,
};
use std::{io, ops::Deref};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd,
    op::{BufResultExt, Recv, RecvManaged, ResultTakeBuffer, Send},
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};
use compio_runtime::{Attacher, BorrowedBuffer, BufferPool};
#[cfg(unix)]
use {
    compio_buf::{IoVectoredBuf, IoVectoredBufMut},
    compio_driver::op::{RecvVectored, SendVectored},
};

/// A wrapper for IO source, providing implementations for [`AsyncRead`] and
/// [`AsyncWrite`].
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

impl<T: AsFd + 'static> AsyncReadManaged for AsyncFd<T> {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        (&*self).read_managed(buffer_pool, len).await
    }
}

impl<T: AsFd + 'static> AsyncReadManaged for &AsyncFd<T> {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        let fd = self.to_shared_fd();
        let buffer_pool = buffer_pool.try_inner()?;
        let op = RecvManaged::new(fd, buffer_pool, len)?;
        compio_runtime::submit_with_extra(op)
            .await
            .take_buffer(buffer_pool)
    }
}

impl<T: AsFd + 'static> AsyncRead for &AsyncFd<T> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        let fd = self.inner.to_shared_fd();
        let op = Recv::new(fd, buf);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }

    #[cfg(unix)]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        let fd = self.inner.to_shared_fd();
        let op = RecvVectored::new(fd, buf);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }
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
        let op = Send::new(fd, buf);
        compio_runtime::submit(op).await.into_inner()
    }

    #[cfg(unix)]
    async fn write_vectored<V: IoVectoredBuf>(&mut self, buf: V) -> BufResult<usize, V> {
        let fd = self.inner.to_shared_fd();
        let op = SendVectored::new(fd, buf);
        compio_runtime::submit(op).await.into_inner()
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

#[cfg(windows)]
impl<T: AsFd + AsRawHandle> AsRawHandle for AsyncFd<T> {
    fn as_raw_handle(&self) -> RawHandle {
        self.inner.as_raw_handle()
    }
}

#[cfg(windows)]
impl<T: AsFd + AsRawSocket> AsRawSocket for AsyncFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.inner.as_raw_socket()
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

#[cfg(unix)]
impl<T: AsFd + FromRawFd> FromRawFd for AsyncFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        unsafe { Self::new_unchecked(FromRawFd::from_raw_fd(fd)) }
    }
}

#[cfg(windows)]
impl<T: AsFd + FromRawHandle> FromRawHandle for AsyncFd<T> {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        unsafe { Self::new_unchecked(FromRawHandle::from_raw_handle(handle)) }
    }
}

#[cfg(windows)]
impl<T: AsFd + FromRawSocket> FromRawSocket for AsyncFd<T> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        unsafe { Self::new_unchecked(FromRawSocket::from_raw_socket(sock)) }
    }
}

impl<T: AsFd> Deref for AsyncFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
