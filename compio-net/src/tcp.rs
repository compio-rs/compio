use std::{future::Future, io, net::SocketAddr};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::impl_raw_fd;
use compio_io::{AsyncRead, AsyncWrite};
use socket2::{Protocol, SockAddr, Type};

use crate::{OwnedReadHalf, OwnedWriteHalf, ReadHalf, Socket, ToSocketAddrsAsync, WriteHalf};

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
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// let addr = "127.0.0.1:2345".parse::<SocketAddr>().unwrap();
///
/// let listener = TcpListener::bind(&addr).await.unwrap();
///
/// let tx_fut = TcpStream::connect(&addr);
///
/// let rx_fut = listener.accept();
///
/// let (mut tx, (mut rx, _)) = futures_util::try_join!(tx_fut, rx_fut).unwrap();
///
/// tx.write_all("test").await.0.unwrap();
///
/// let (_, buf) = rx.read_exact(Vec::with_capacity(4)).await.unwrap();
///
/// assert_eq!(buf, b"test");
/// # });
/// ```
#[derive(Debug, Clone)]
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
    pub async fn bind(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        super::each_addr(addr, |addr| async move {
            let socket =
                Socket::bind(&SockAddr::from(addr), Type::STREAM, Some(Protocol::TCP)).await?;
            socket.listen(128)?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Close the socket. If the returned future is dropped before polling, the
    /// socket won't be closed.
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
    }

    /// Accepts a new incoming connection from this listener.
    ///
    /// This function will yield once a new TCP connection is established. When
    /// established, the corresponding [`TcpStream`] and the remote peer's
    /// address will be returned.
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
    /// # compio_runtime::Runtime::new().unwrap().block_on(async {
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

impl_raw_fd!(TcpListener, inner, socket);

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
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// // Connect to a peer
/// let mut stream = TcpStream::connect("127.0.0.1:8080").await.unwrap();
///
/// // Write some data.
/// stream.write("hello world!").await.unwrap();
/// # })
/// ```
#[derive(Debug, Clone)]
pub struct TcpStream {
    inner: Socket,
}

impl TcpStream {
    /// Opens a TCP connection to a remote host.
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
                Socket::bind(&bind_addr, Type::STREAM, Some(Protocol::TCP)).await?
            } else {
                Socket::new(addr2.domain(), Type::STREAM, Some(Protocol::TCP)).await?
            };
            socket.connect_async(&addr2).await?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Close the socket. If the returned future is dropped before polling, the
    /// socket won't be closed.
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
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

    /// Splits a [`TcpStream`] into a read half and a write half, which can be
    /// used to read and write the stream concurrently.
    ///
    /// This method is more efficient than
    /// [`into_split`](TcpStream::into_split), but the halves cannot
    /// be moved into independently spawned tasks.
    pub fn split(&self) -> (ReadHalf<Self>, WriteHalf<Self>) {
        crate::split(self)
    }

    /// Splits a [`TcpStream`] into a read half and a write half, which can be
    /// used to read and write the stream concurrently.
    ///
    /// Unlike [`split`](TcpStream::split), the owned halves can be moved to
    /// separate tasks, however this comes at the cost of a heap allocation.
    pub fn into_split(self) -> (OwnedReadHalf<Self>, OwnedWriteHalf<Self>) {
        crate::into_split(self)
    }
}

impl AsyncRead for TcpStream {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).read(buf).await
    }

    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        (&*self).read_vectored(buf).await
    }
}

impl AsyncRead for &TcpStream {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.inner.recv(buf).await
    }

    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.inner.recv_vectored(buf).await
    }
}

impl AsyncWrite for TcpStream {
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

impl AsyncWrite for &TcpStream {
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

impl_raw_fd!(TcpStream, inner, socket);
