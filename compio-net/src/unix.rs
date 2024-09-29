use std::{future::Future, io, path::Path};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::impl_raw_fd;
use compio_io::{AsyncRead, AsyncWrite};
use socket2::{SockAddr, Socket as Socket2, Type};

use crate::{OwnedReadHalf, OwnedWriteHalf, PollFd, ReadHalf, Socket, WriteHalf};

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
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// let listener = UnixListener::bind(&sock_file).await.unwrap();
///
/// let (mut tx, (mut rx, _)) =
///     futures_util::try_join!(UnixStream::connect(&sock_file), listener.accept()).unwrap();
///
/// tx.write_all("test").await.0.unwrap();
///
/// let (_, buf) = rx.read_exact(Vec::with_capacity(4)).await.unwrap();
///
/// assert_eq!(buf, b"test");
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct UnixListener {
    inner: Socket,
}

impl UnixListener {
    /// Creates a new [`UnixListener`], which will be bound to the specified
    /// file path. The file path cannot yet exist, and will be cleaned up
    /// upon dropping [`UnixListener`]
    pub async fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::bind_addr(&SockAddr::unix(path)?).await
    }

    /// Creates a new [`UnixListener`] with [`SockAddr`], which will be bound to
    /// the specified file path. The file path cannot yet exist, and will be
    /// cleaned up upon dropping [`UnixListener`]
    pub async fn bind_addr(addr: &SockAddr) -> io::Result<Self> {
        if !addr.is_unix() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "addr is not unix socket address",
            ));
        }

        let socket = Socket::bind(addr, Type::STREAM, None).await?;
        socket.listen(1024)?;
        Ok(UnixListener { inner: socket })
    }

    /// Close the socket. If the returned future is dropped before polling, the
    /// socket won't be closed.
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
    }

    /// Accepts a new incoming connection from this listener.
    ///
    /// This function will yield once a new Unix domain socket connection
    /// is established. When established, the corresponding [`UnixStream`] and
    /// will be returned.
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

impl_raw_fd!(UnixListener, socket2::Socket, inner, socket);

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
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// // Connect to a peer
/// let mut stream = UnixStream::connect("unix-server.sock").await.unwrap();
///
/// // Write some data.
/// stream.write("hello world!").await.unwrap();
/// # })
/// ```
#[derive(Debug, Clone)]
pub struct UnixStream {
    inner: Socket,
}

impl UnixStream {
    /// Opens a Unix connection to the specified file path. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a `UnixStream`.
    pub async fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::connect_addr(&SockAddr::unix(path)?).await
    }

    /// Opens a Unix connection to the specified address. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a `UnixStream`.
    pub async fn connect_addr(addr: &SockAddr) -> io::Result<Self> {
        if !addr.is_unix() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "addr is not unix socket address",
            ));
        }

        #[cfg(windows)]
        let socket = {
            let new_addr = empty_unix_socket();
            Socket::bind(&new_addr, Type::STREAM, None).await?
        };
        #[cfg(unix)]
        let socket = {
            use socket2::Domain;
            Socket::new(Domain::UNIX, Type::STREAM, None).await?
        };
        socket.connect_async(addr).await?;
        let unix_stream = UnixStream { inner: socket };
        Ok(unix_stream)
    }

    #[cfg(unix)]
    /// Creates new UnixStream from a std::os::unix::net::UnixStream.
    pub fn from_std(stream: std::os::unix::net::UnixStream) -> io::Result<Self> {
        Ok(Self {
            inner: Socket::from_socket2(Socket2::from(stream))?,
        })
    }

    /// Close the socket. If the returned future is dropped before polling, the
    /// socket won't be closed.
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
    }

    /// Returns the socket path of the remote peer of this connection.
    pub fn peer_addr(&self) -> io::Result<SockAddr> {
        #[allow(unused_mut)]
        let mut addr = self.inner.peer_addr()?;
        #[cfg(windows)]
        {
            fix_unix_socket_length(&mut addr);
        }
        Ok(addr)
    }

    /// Returns the socket path of the local half of this connection.
    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.inner.local_addr()
    }

    /// Splits a [`UnixStream`] into a read half and a write half, which can be
    /// used to read and write the stream concurrently.
    ///
    /// This method is more efficient than
    /// [`into_split`](UnixStream::into_split), but the halves cannot
    /// be moved into independently spawned tasks.
    pub fn split(&self) -> (ReadHalf<Self>, WriteHalf<Self>) {
        crate::split(self)
    }

    /// Splits a [`UnixStream`] into a read half and a write half, which can be
    /// used to read and write the stream concurrently.
    ///
    /// Unlike [`split`](UnixStream::split), the owned halves can be moved to
    /// separate tasks, however this comes at the cost of a heap allocation.
    pub fn into_split(self) -> (OwnedReadHalf<Self>, OwnedWriteHalf<Self>) {
        crate::into_split(self)
    }

    /// Create [`PollFd`] from inner socket.
    pub fn to_poll_fd(&self) -> io::Result<PollFd<Socket2>> {
        self.inner.to_poll_fd()
    }

    /// Create [`PollFd`] from inner socket.
    pub fn into_poll_fd(self) -> io::Result<PollFd<Socket2>> {
        self.inner.into_poll_fd()
    }
}

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
        self.inner.shutdown().await
    }
}

impl_raw_fd!(UnixStream, socket2::Socket, inner, socket);

#[cfg(windows)]
#[inline]
fn empty_unix_socket() -> SockAddr {
    use windows_sys::Win32::Networking::WinSock::{AF_UNIX, SOCKADDR_UN};

    // SAFETY: the length is correct
    unsafe {
        SockAddr::try_init(|addr, len| {
            let addr: *mut SOCKADDR_UN = addr.cast();
            std::ptr::write(addr, SOCKADDR_UN {
                sun_family: AF_UNIX,
                sun_path: [0; 108],
            });
            std::ptr::write(len, 3);
            Ok(())
        })
    }
    // it is always Ok
    .unwrap()
    .1
}

// The peer addr returned after ConnectEx is buggy. It contains bytes that
// should not belong to the address. Luckily a unix path should not contain `\0`
// until the end. We can determine the path ending by that.
#[cfg(windows)]
#[inline]
fn fix_unix_socket_length(addr: &mut SockAddr) {
    use windows_sys::Win32::Networking::WinSock::SOCKADDR_UN;

    // SAFETY: cannot construct non-unix socket address in safe way.
    let unix_addr: &SOCKADDR_UN = unsafe { &*addr.as_ptr().cast() };
    let addr_len = match std::ffi::CStr::from_bytes_until_nul(&unix_addr.sun_path) {
        Ok(str) => str.to_bytes_with_nul().len() + 2,
        Err(_) => std::mem::size_of::<SOCKADDR_UN>(),
    };
    unsafe {
        addr.set_length(addr_len as _);
    }
}
