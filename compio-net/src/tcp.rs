use std::{future::Future, io, net::SocketAddr};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::impl_raw_fd;
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite, util::Splittable};
use compio_runtime::{BorrowedBuffer, BufferPool};
use socket2::{Protocol, SockAddr, Socket as Socket2, Type};

use crate::{
    OwnedReadHalf, OwnedWriteHalf, PollFd, ReadHalf, Socket, SocketOpts, ToSocketAddrsAsync,
    WriteHalf,
};

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
/// let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
///
/// let addr = listener.local_addr().unwrap();
///
/// let tx_fut = TcpStream::connect(&addr);
///
/// let rx_fut = listener.accept();
///
/// let (mut tx, (mut rx, _)) = futures_util::try_join!(tx_fut, rx_fut).unwrap();
///
/// tx.write_all("test").await.0.unwrap();
///
/// let (_, mut buf) = rx.read_exact(Vec::with_capacity(4)).await.unwrap();
/// unsafe { buf.set_len(4) };
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
    ///
    /// It enables the `SO_REUSEADDR` option by default.
    pub async fn bind(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        Self::bind_with_options(addr, &SocketOpts::default().reuse_address(true)).await
    }

    /// Creates a new `TcpListener`, which will be bound to the specified
    /// address using `SocketOpts`.
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port
    /// to this listener.
    pub async fn bind_with_options(
        addr: impl ToSocketAddrsAsync,
        options: &SocketOpts,
    ) -> io::Result<Self> {
        super::each_addr(addr, |addr| async move {
            let sa = SockAddr::from(addr);
            let socket = Socket::new(sa.domain(), Type::STREAM, Some(Protocol::TCP)).await?;
            options.setup_socket(&socket)?;
            socket.socket.bind(&sa)?;
            socket.listen(128)?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Creates new TcpListener from a [`std::net::TcpListener`].
    pub fn from_std(stream: std::net::TcpListener) -> io::Result<Self> {
        Ok(Self {
            inner: Socket::from_socket2(Socket2::from(stream))?,
        })
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
        self.accept_with_options(&SocketOpts::default()).await
    }

    /// Accepts a new incoming connection from this listener, and sets options.
    ///
    /// This function will yield once a new TCP connection is established. When
    /// established, the corresponding [`TcpStream`] and the remote peer's
    /// address will be returned.
    pub async fn accept_with_options(
        &self,
        options: &SocketOpts,
    ) -> io::Result<(TcpStream, SocketAddr)> {
        let (socket, addr) = self.inner.accept().await?;
        options.setup_socket(&socket)?;
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

impl_raw_fd!(TcpListener, socket2::Socket, inner, socket);

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
        Self::connect_with_options(addr, &SocketOpts::default()).await
    }

    /// Opens a TCP connection to a remote host using `SocketOpts`.
    pub async fn connect_with_options(
        addr: impl ToSocketAddrsAsync,
        options: &SocketOpts,
    ) -> io::Result<Self> {
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
            options.setup_socket(&socket)?;
            socket.connect_async(&addr2).await?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Bind to `bind_addr` then opens a TCP connection to a remote host.
    pub async fn bind_and_connect(
        bind_addr: SocketAddr,
        addr: impl ToSocketAddrsAsync,
    ) -> io::Result<Self> {
        Self::bind_and_connect_with_options(bind_addr, addr, &SocketOpts::default()).await
    }

    /// Bind to `bind_addr` then opens a TCP connection to a remote host using
    /// `SocketOpts`.
    pub async fn bind_and_connect_with_options(
        bind_addr: SocketAddr,
        addr: impl ToSocketAddrsAsync,
        options: &SocketOpts,
    ) -> io::Result<Self> {
        super::each_addr(addr, |addr| async move {
            let addr = SockAddr::from(addr);
            let bind_addr = SockAddr::from(bind_addr);

            let socket = Socket::bind(&bind_addr, Type::STREAM, Some(Protocol::TCP)).await?;
            options.setup_socket(&socket)?;
            socket.connect_async(&addr).await?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Creates new TcpStream from a [`std::net::TcpStream`].
    pub fn from_std(stream: std::net::TcpStream) -> io::Result<Self> {
        Ok(Self {
            inner: Socket::from_socket2(Socket2::from(stream))?,
        })
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
    pub fn split(&self) -> (ReadHalf<'_, Self>, WriteHalf<'_, Self>) {
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

    /// Create [`PollFd`] from inner socket.
    pub fn to_poll_fd(&self) -> io::Result<PollFd<Socket2>> {
        self.inner.to_poll_fd()
    }

    /// Create [`PollFd`] from inner socket.
    pub fn into_poll_fd(self) -> io::Result<PollFd<Socket2>> {
        self.inner.into_poll_fd()
    }

    /// Gets the value of the `TCP_NODELAY` option on this socket.
    ///
    /// For more information about this option, see
    /// [`TcpStream::set_nodelay`].
    pub fn nodelay(&self) -> io::Result<bool> {
        self.inner.socket.tcp_nodelay()
    }

    /// Sets the value of the TCP_NODELAY option on this socket.
    ///
    /// If set, this option disables the Nagle algorithm. This means
    /// that segments are always sent as soon as possible, even if
    /// there is only a small amount of data. When not set, data is
    /// buffered until there is a sufficient amount to send out,
    /// thereby avoiding the frequent sending of small packets.
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.inner.socket.set_tcp_nodelay(nodelay)
    }

    /// Sends out-of-band data on this socket.
    ///
    /// Out-of-band data is sent with the `MSG_OOB` flag.
    pub async fn send_out_of_band<T: IoBuf>(&self, buf: T) -> BufResult<usize, T> {
        #[cfg(unix)]
        use libc::MSG_OOB;
        #[cfg(windows)]
        use windows_sys::Win32::Networking::WinSock::MSG_OOB;

        self.inner.send(buf, MSG_OOB).await
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
        self.inner.recv(buf, 0).await
    }

    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.inner.recv_vectored(buf, 0).await
    }
}

impl AsyncReadManaged for TcpStream {
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

impl AsyncReadManaged for &TcpStream {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        self.inner.recv_managed(buffer_pool, len as _, 0).await
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
        self.inner.send(buf, 0).await
    }

    #[inline]
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.inner.send_vectored(buf, 0).await
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

impl Splittable for TcpStream {
    type ReadHalf = OwnedReadHalf<Self>;
    type WriteHalf = OwnedWriteHalf<Self>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        crate::into_split(self)
    }
}

impl<'a> Splittable for &'a TcpStream {
    type ReadHalf = ReadHalf<'a, TcpStream>;
    type WriteHalf = WriteHalf<'a, TcpStream>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        crate::split(self)
    }
}

impl_raw_fd!(TcpStream, socket2::Socket, inner, socket);
