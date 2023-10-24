use std::io;

use compio_driver::impl_raw_fd;
use socket2::SockAddr;
#[cfg(feature = "runtime")]
use {
    compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut},
    compio_io::{AsyncRead, AsyncWrite},
    compio_runtime::impl_attachable,
    socket2::{Domain, Type},
    std::{future::Future, path::Path},
};

use crate::Socket;

/// A Unix socket server, listening for connections.
///
/// You can accept a new connection by using the [`UnixListener::accept`]
/// method.
///
/// # Examples
///
/// ```
/// use compio_io::{AsyncReadExt, AsyncWriteExt};
/// use compio_net::{UnixListener, UnixStream};
/// use tempfile::tempdir;
///
/// let dir = tempdir().unwrap();
/// let sock_file = dir.path().join("unix-server.sock");
///
/// compio_runtime::block_on(async move {
///     let listener = UnixListener::bind(&sock_file).await.unwrap();
///
///     let mut tx = UnixStream::connect(&sock_file).await.unwrap();
///     let (mut rx, _) = listener.accept().await.unwrap();
///
///     tx.write_all("test").await.0.unwrap();
///
///     let (_, buf) = rx.read_exact(Vec::with_capacity(4)).await.unwrap();
///
///     assert_eq!(buf, b"test");
/// });
/// ```
#[derive(Debug)]
pub struct UnixListener {
    inner: Socket,
}

impl UnixListener {
    /// Creates a new [`UnixListener`], which will be bound to the specified
    /// file path. The file path cannot yet exist, and will be cleaned up
    /// upon dropping [`UnixListener`]
    #[cfg(feature = "runtime")]
    pub async fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::bind_addr(&SockAddr::unix(path)?).await
    }

    /// Creates a new [`UnixListener`] with [`SockAddr`], which will be bound to
    /// the specified file path. The file path cannot yet exist, and will be
    /// cleaned up upon dropping [`UnixListener`]
    #[cfg(feature = "runtime")]
    pub async fn bind_addr(addr: &SockAddr) -> io::Result<Self> {
        let socket = Socket::bind(addr, Type::STREAM, None).await?;
        socket.listen(1024)?;
        Ok(UnixListener { inner: socket })
    }

    /// Close the socket. If the returned future is dropped before polling, the
    /// socket won't be closed.
    #[cfg(feature = "runtime")]
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
    }

    /// Creates a new independently owned handle to the underlying socket.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            inner: self.inner.try_clone()?,
        })
    }

    /// Accepts a new incoming connection from this listener.
    ///
    /// This function will yield once a new Unix domain socket connection
    /// is established. When established, the corresponding [`UnixStream`] and
    /// will be returned.
    #[cfg(feature = "runtime")]
    pub async fn accept(&self) -> io::Result<(UnixStream, SockAddr)> {
        let (socket, addr) = self.inner.accept().await?;
        let stream = UnixStream { inner: socket };
        Ok((stream, addr))
    }

    /// Returns the local address that this listener is bound to.
    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.inner.local_addr()
    }
}

impl_raw_fd!(UnixListener, inner);

#[cfg(feature = "runtime")]
impl_attachable!(UnixListener, inner);

/// A Unix stream between two local sockets on Windows & WSL.
///
/// A Unix stream can either be created by connecting to an endpoint, via the
/// `connect` method, or by accepting a connection from a listener.
///
/// # Examples
///
/// ```no_run
/// use compio_io::AsyncWrite;
/// use compio_net::UnixStream;
///
/// compio_runtime::block_on(async {
///     // Connect to a peer
///     let mut stream = UnixStream::connect("unix-server.sock").await.unwrap();
///
///     // Write some data.
///     stream.write("hello world!").await.unwrap();
/// })
/// ```
#[derive(Debug)]
pub struct UnixStream {
    inner: Socket,
}

impl UnixStream {
    /// Opens a Unix connection to the specified file path. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a `UnixStream`.
    #[cfg(feature = "runtime")]
    pub async fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::connect_addr(&SockAddr::unix(path)?).await
    }

    /// Opens a Unix connection to the specified address. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a `UnixStream`.
    #[cfg(feature = "runtime")]
    pub async fn connect_addr(addr: &SockAddr) -> io::Result<Self> {
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None).await?;
        socket.connect(addr)?;
        let unix_stream = UnixStream { inner: socket };
        Ok(unix_stream)
    }

    /// Close the socket. If the returned future is dropped before polling, the
    /// socket won't be closed.
    #[cfg(feature = "runtime")]
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
    }

    /// Creates a new independently owned handle to the underlying socket.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            inner: self.inner.try_clone()?,
        })
    }

    /// Returns the socket path of the remote peer of this connection.
    pub fn peer_addr(&self) -> io::Result<SockAddr> {
        self.inner.peer_addr()
    }

    /// Returns the socket path of the local half of this connection.
    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.inner.local_addr()
    }
}

#[cfg(feature = "runtime")]
impl AsyncRead for UnixStream {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).read(buf).await
    }

    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        (&*self).read_vectored(buf).await
    }
}

#[cfg(feature = "runtime")]
impl AsyncRead for &UnixStream {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.inner.recv(buf).await
    }

    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.inner.recv_vectored(buf).await
    }
}

#[cfg(feature = "runtime")]
impl AsyncWrite for UnixStream {
    #[inline]
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (&*self).write(buf).await
    }

    #[inline]
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
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

#[cfg(feature = "runtime")]
impl AsyncWrite for &UnixStream {
    #[inline]
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.inner.send(buf).await
    }

    #[inline]
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.inner.send_vectored(buf).await
    }

    #[inline]
    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    async fn shutdown(&mut self) -> io::Result<()> {
        self.inner.shutdown()
    }
}

impl_raw_fd!(UnixStream, inner);

#[cfg(feature = "runtime")]
impl_attachable!(UnixStream, inner);
