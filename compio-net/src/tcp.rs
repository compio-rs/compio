use std::{io, net::SocketAddr};

use compio_driver::impl_raw_fd;
#[cfg(feature = "runtime")]
use {
    crate::ToSocketAddrsAsync,
    compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut},
    compio_io::{AsyncRead, AsyncWrite},
    compio_runtime::impl_attachable,
    socket2::{Protocol, SockAddr, Type},
};

use crate::Socket;

/// A TCP socket server, listening for connections.
///
/// You can accept a new connection by using the
/// [`accept`](`TcpListener::accept`) method.
///
/// # Examples
///
/// ```
/// use std::net::SocketAddr;
///
/// use compio_io::{AsyncReadExt, AsyncWriteExt};
/// use compio_net::{TcpListener, TcpStream};
/// use socket2::SockAddr;
///
/// let addr = "127.0.0.1:2345".parse::<SocketAddr>().unwrap();
///
/// compio_runtime::block_on(async move {
///     let listener = TcpListener::bind(&addr).await.unwrap();
///
///     let tx_fut = TcpStream::connect(&addr);
///
///     let rx_fut = listener.accept();
///
///     let (mut tx, (mut rx, _)) = futures_util::try_join!(tx_fut, rx_fut).unwrap();
///
///     tx.write_all("test").await.0.unwrap();
///
///     let (_, buf) = rx.read_exact(Vec::with_capacity(4)).await.unwrap();
///
///     assert_eq!(buf, b"test");
/// });
/// ```
#[derive(Debug)]
pub struct TcpListener {
    inner: Socket,
}

impl TcpListener {
    /// Creates a new `TcpListener`, which will be bound to the specified
    /// address.
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port
    /// to this listener.
    #[cfg(feature = "runtime")]
    pub async fn bind(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        super::each_addr(addr, |addr| async move {
            let socket = Socket::bind(&SockAddr::from(addr), Type::STREAM, Some(Protocol::TCP))?;
            socket.listen(128)?;
            Ok(Self { inner: socket })
        })
        .await
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
    /// This function will yield once a new TCP connection is established. When
    /// established, the corresponding [`TcpStream`] and the remote peer's
    /// address will be returned.
    #[cfg(feature = "runtime")]
    pub async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (socket, addr) = self.inner.accept().await?;
        let stream = TcpStream { inner: socket };
        Ok((stream, addr.as_socket().expect("should be SocketAddr")))
    }

    /// Returns the local address that this listener is bound to.
    ///
    /// This can be useful, for example, when binding to port 0 to
    /// figure out which port was actually bound.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    ///
    /// use compio_net::TcpListener;
    /// use socket2::SockAddr;
    ///
    /// # compio_runtime::block_on(async {
    /// let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();
    ///
    /// let addr = listener.local_addr().expect("Couldn't get local address");
    /// assert_eq!(
    ///     addr,
    ///     SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080))
    /// );
    /// # });
    /// ```
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner
            .local_addr()
            .map(|addr| addr.as_socket().expect("should be SocketAddr"))
    }
}

impl_raw_fd!(TcpListener, inner);

#[cfg(feature = "runtime")]
impl_attachable!(TcpListener, inner);

/// A TCP stream between a local and a remote socket.
///
/// A TCP stream can either be created by connecting to an endpoint, via the
/// `connect` method, or by accepting a connection from a listener.
///
/// # Examples
///
/// ```no_run
/// use std::net::SocketAddr;
///
/// use compio_io::AsyncWrite;
/// use compio_net::TcpStream;
///
/// compio_runtime::block_on(async {
///     // Connect to a peer
///     let mut stream = TcpStream::connect("127.0.0.1:8080").await.unwrap();
///
///     // Write some data.
///     stream.write("hello world!").await.unwrap();
/// })
/// ```
#[derive(Debug)]
pub struct TcpStream {
    inner: Socket,
}

impl TcpStream {
    /// Opens a TCP connection to a remote host.
    #[cfg(feature = "runtime")]
    pub async fn connect(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

        super::each_addr(addr, |addr| async move {
            let addr2 = SockAddr::from(addr);
            let socket = if cfg!(windows) {
                let bind_addr = if addr.is_ipv4() {
                    SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
                } else if addr.is_ipv6() {
                    SockAddr::from(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrNotAvailable,
                        "Unsupported address domain.",
                    ));
                };
                Socket::bind(&bind_addr, Type::STREAM, Some(Protocol::TCP))?
            } else {
                Socket::new(addr2.domain(), Type::STREAM, Some(Protocol::TCP))?
            };
            socket.connect_async(&addr2).await?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Creates a new independently owned handle to the underlying socket.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            inner: self.inner.try_clone()?,
        })
    }

    /// Returns the socket address of the remote peer of this TCP connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.inner
            .peer_addr()
            .map(|addr| addr.as_socket().expect("should be SocketAddr"))
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner
            .local_addr()
            .map(|addr| addr.as_socket().expect("should be SocketAddr"))
    }
}

#[cfg(feature = "runtime")]
impl AsyncRead for TcpStream {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.inner.read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.inner.read_vectored(buf).await
    }
}

#[cfg(feature = "runtime")]
impl AsyncWrite for TcpStream {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.inner.write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.inner.write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.inner.shutdown().await
    }
}

impl_raw_fd!(TcpStream, inner);

#[cfg(feature = "runtime")]
impl_attachable!(TcpStream, inner);
