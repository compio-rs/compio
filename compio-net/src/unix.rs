use std::{io, net::Shutdown, path::Path};

#[cfg(feature = "runtime")]
use ::compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::impl_raw_fd;
use socket2::{Domain, SockAddr, Type};

use crate::{Socket, ToSockAddrs};

/// A Unix socket server, listening for connections.
///
/// You can accept a new connection by using the [`UnixListener::accept`]
/// method.
///
/// # Examples
///
/// ```
/// use compio::net::{UnixListener, UnixStream};
/// use tempfile::tempdir;
///
/// let dir = tempdir().unwrap();
/// let sock_file = dir.path().join("unix-server.sock");
///
/// compio::task::block_on(async move {
///     let listener = UnixListener::bind(&sock_file).unwrap();
///
///     let tx = UnixStream::connect(&sock_file).unwrap();
///     let (rx, _) = listener.accept().await.unwrap();
///
///     tx.send_all("test").await.0.unwrap();
///
///     let (res, buf) = rx.recv_exact(Vec::with_capacity(4)).await;
///     res.unwrap();
///
///     assert_eq!(buf, b"test");
/// });
/// ```
pub struct UnixListener {
    inner: Socket,
}

impl UnixListener {
    /// Creates a new [`UnixListener`], which will be bound to the specified
    /// file path. The file path cannot yet exist, and will be cleaned up
    /// upon dropping [`UnixListener`]
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::bind_addr(SockAddr::unix(path)?)
    }

    /// Creates a new [`UnixListener`] with [`SockAddr`], which will be bound to
    /// the specified file path. The file path cannot yet exist, and will be
    /// cleaned up upon dropping [`UnixListener`]
    pub fn bind_addr(addr: impl ToSockAddrs) -> io::Result<Self> {
        super::each_addr(addr, |addr| {
            let socket = Socket::bind(&addr, Type::STREAM, None)?;
            socket.listen(1024)?;
            Ok(UnixListener { inner: socket })
        })
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

/// A Unix stream between two local sockets on Windows & WSL.
///
/// A Unix stream can either be created by connecting to an endpoint, via the
/// `connect` method, or by accepting a connection from a listener.
///
/// # Examples
///
/// ```no_run
/// use compio::net::UnixStream;
///
/// compio::task::block_on(async {
///     // Connect to a peer
///     let mut stream = UnixStream::connect("unix-server.sock").unwrap();
///
///     // Write some data.
///     let (result, _) = stream.send("hello world!").await;
///     result.unwrap();
/// })
/// ```
pub struct UnixStream {
    inner: Socket,
}

impl UnixStream {
    /// Opens a Unix connection to the specified file path. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a `UnixStream`.
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::connect_addr(SockAddr::unix(path)?)
    }

    /// Opens a Unix connection to the specified address. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a `UnixStream`.
    pub fn connect_addr(addr: impl ToSockAddrs) -> io::Result<Self> {
        super::each_addr(addr, |addr| {
            let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
            socket.connect(&addr)?;
            let unix_stream = UnixStream { inner: socket };
            Ok(unix_stream)
        })
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

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O on the specified
    /// portions to return immediately with an appropriate value (see the
    /// documentation of [`Shutdown`]).
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.inner.shutdown(how)
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    #[cfg(feature = "runtime")]
    pub async fn recv<T: IoBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv(buffer).await
    }

    /// Receives exact number of bytes from the socket.
    #[cfg(feature = "runtime")]
    pub async fn recv_exact<T: IoBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv_exact(buffer).await
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    #[cfg(feature = "runtime")]
    pub async fn recv_vectored<T: IoVectoredBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv_vectored(buffer).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    #[cfg(feature = "runtime")]
    pub async fn send<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send(buffer).await
    }

    /// Sends all data to the socket.
    #[cfg(feature = "runtime")]
    pub async fn send_all<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send_all(buffer).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    #[cfg(feature = "runtime")]
    pub async fn send_vectored<T: IoVectoredBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send_vectored(buffer).await
    }
}

impl_raw_fd!(UnixStream, inner);
