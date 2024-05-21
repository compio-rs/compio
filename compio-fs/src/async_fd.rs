use std::io;
#[cfg(unix)]
use std::os::fd::{FromRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{
    AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, RawHandle, RawSocket,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    op::{BufResultExt, Recv, Send},
    AsRawFd, SharedFd, ToSharedFd,
};
use compio_io::{AsyncRead, AsyncWrite};
use compio_runtime::Attacher;
#[cfg(unix)]
use {
    compio_buf::{IoVectoredBuf, IoVectoredBufMut},
    compio_driver::op::{RecvVectored, SendVectored},
};

/// A wrapper for IO source, providing implementations for [`AsyncRead`] and
/// [`AsyncWrite`].
#[derive(Debug)]
pub struct AsyncFd<T: AsRawFd> {
    inner: Attacher<T>,
}

impl<T: AsRawFd> AsyncFd<T> {
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
    /// The user should handle the attachment correctly.
    pub unsafe fn new_unchecked(source: T) -> Self {
        Self {
            inner: Attacher::new_unchecked(source),
        }
    }
}

impl<T: AsRawFd + 'static> AsyncRead for AsyncFd<T> {
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

impl<T: AsRawFd + 'static> AsyncRead for &AsyncFd<T> {
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

impl<T: AsRawFd + 'static> AsyncWrite for AsyncFd<T> {
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

impl<T: AsRawFd + 'static> AsyncWrite for &AsyncFd<T> {
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

impl<T: AsRawFd> IntoInner for AsyncFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.inner.into_inner()
    }
}

impl<T: AsRawFd> AsRawFd for AsyncFd<T> {
    fn as_raw_fd(&self) -> compio_driver::RawFd {
        self.inner.as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: AsRawFd + AsRawHandle> AsRawHandle for AsyncFd<T> {
    fn as_raw_handle(&self) -> RawHandle {
        self.inner.as_raw_handle()
    }
}

#[cfg(windows)]
impl<T: AsRawFd + AsRawSocket> AsRawSocket for AsyncFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.inner.as_raw_socket()
    }
}

impl<T: AsRawFd> ToSharedFd<T> for AsyncFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.inner.to_shared_fd()
    }
}

impl<T: AsRawFd> Clone for AsyncFd<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(unix)]
impl<T: AsRawFd + FromRawFd> FromRawFd for AsyncFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new_unchecked(FromRawFd::from_raw_fd(fd))
    }
}

#[cfg(windows)]
impl<T: AsRawFd + FromRawHandle> FromRawHandle for AsyncFd<T> {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        Self::new_unchecked(FromRawHandle::from_raw_handle(handle))
    }
}

#[cfg(windows)]
impl<T: AsRawFd + FromRawSocket> FromRawSocket for AsyncFd<T> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        Self::new_unchecked(FromRawSocket::from_raw_socket(sock))
    }
}
