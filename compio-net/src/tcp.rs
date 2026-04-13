use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::{BufferRef, impl_raw_fd, op::RecvMsgMultiResult};
use compio_io::{
    AsyncRead, AsyncReadManaged, AsyncReadMulti, AsyncWrite,
    ancillary::{
        AsyncReadAncillary, AsyncReadAncillaryManaged, AsyncReadAncillaryMulti, AsyncWriteAncillary,
    },
    util::Splittable,
};
use compio_runtime::fd::PollFd;
use futures_util::{Stream, StreamExt, stream::FusedStream};
use socket2::{Protocol, SockAddr, Socket as Socket2, Type};

use crate::{
    Incoming, MSG_NOSIGNAL, OwnedReadHalf, OwnedWriteHalf, ReadHalf, Socket, ToSocketAddrsAsync,
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
    ///
    /// It enables the `SO_REUSEADDR` option by default.
    ///
    /// To configure the socket before binding, you can use the [`TcpSocket`]
    /// type.
    pub async fn bind(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        super::each_addr(addr, |addr| async move {
            let sa = SockAddr::from(addr);
            let socket = Socket::new(sa.domain(), Type::STREAM, Some(Protocol::TCP)).await?;
            socket.socket.set_reuse_address(true)?;
            socket.bind(&sa).await?;
            socket.listen(128).await?;
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
    ///
    /// See [`TcpStream::close`] for more details.
    ///
    /// [`TcpStream::close`]: crate::tcp::TcpStream::close
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

    /// Returns a stream of incoming connections to this listener.
    pub fn incoming(&self) -> TcpIncoming<'_> {
        TcpIncoming {
            inner: self.inner.incoming(),
        }
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

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    ///
    /// For more information about this option, see [`set_ttl_v4`].
    ///
    /// [`set_ttl_v4`]: method@Self::set_ttl_v4
    pub fn ttl_v4(&self) -> io::Result<u32> {
        self.inner.socket.ttl_v4()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    ///
    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl_v4(&self, ttl: u32) -> io::Result<()> {
        self.inner.socket.set_ttl_v4(ttl)
    }
}

impl_raw_fd!(TcpListener, socket2::Socket, inner, socket);

/// A stream of incoming TCP connections.
pub struct TcpIncoming<'a> {
    inner: Incoming<'a>,
}

impl Stream for TcpIncoming<'_> {
    type Item = io::Result<TcpStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.inner.poll_next_unpin(cx).map(|res| {
            res.map(|res| {
                let socket = res?;
                Ok(TcpStream { inner: socket })
            })
        })
    }
}

impl FusedStream for TcpIncoming<'_> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

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
    ///
    /// To configure the socket before connecting, you can use the [`TcpSocket`]
    /// type.
    pub async fn connect(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

        super::each_addr(addr, |addr| async move {
            let addr2 = SockAddr::from(addr);
            let socket = Socket::new(addr2.domain(), Type::STREAM, Some(Protocol::TCP)).await?;
            if cfg!(windows) {
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
                socket.bind(&bind_addr).await?;
            };
            socket.connect_async(&addr2).await?;
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
    ///
    /// As the socket is clonable, users can call `close` on a clone, but the
    /// future will never complete until all clones are dropped. Some
    /// operations may keep a strong reference to the socket, so the future
    /// may never complete if there are pending operations.
    ///
    /// It's OK to drop the socket directly without calling `close`, but the
    /// socket may not be closed immediately.
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

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
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

    /// Gets the value of the `TCP_QUICKACK` option on this socket.
    ///
    /// For more information about this option, see [`TcpStream::set_quickack`].
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "fuchsia",
        target_os = "cygwin",
    ))]
    pub fn quickack(&self) -> io::Result<bool> {
        self.inner.socket.tcp_quickack()
    }

    /// Enable or disable `TCP_QUICKACK`.
    ///
    /// This flag causes Linux to eagerly send `ACK`s rather than delaying them.
    /// Linux may reset this flag after further operations on the socket.
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "fuchsia",
        target_os = "cygwin",
    ))]
    pub fn set_quickack(&self, quickack: bool) -> io::Result<()> {
        self.inner.socket.set_tcp_quickack(quickack)
    }

    /// Reads the linger duration for this socket by getting the `SO_LINGER`
    /// option.
    pub fn linger(&self) -> io::Result<Option<Duration>> {
        self.inner.socket.linger()
    }

    /// Sets a linger duration of zero on this socket by setting the `SO_LINGER`
    /// option.
    pub fn set_zero_linger(&self) -> io::Result<()> {
        self.inner.socket.set_linger(Some(Duration::ZERO))
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    ///
    /// For more information about this option, see [`set_ttl_v4`].
    ///
    /// [`set_ttl_v4`]: TcpStream::set_ttl_v4
    pub fn ttl_v4(&self) -> io::Result<u32> {
        self.inner.socket.ttl_v4()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    ///
    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl_v4(&self, ttl: u32) -> io::Result<()> {
        self.inner.socket.set_ttl_v4(ttl)
    }

    /// Sends out-of-band data on this socket.
    ///
    /// Out-of-band data is sent with the `MSG_OOB` flag.
    pub async fn send_out_of_band<T: IoBuf>(&self, buf: T) -> BufResult<usize, T> {
        #[cfg(unix)]
        use libc::MSG_OOB;
        #[cfg(windows)]
        use windows_sys::Win32::Networking::WinSock::MSG_OOB;

        self.inner.send(buf, MSG_OOB | MSG_NOSIGNAL).await
    }

    /// Sends data using [zero-copy send](https://man7.org/linux/man-pages/man3/io_uring_prep_send_zc.3.html).
    ///
    /// If the underlying platform doesn't support zero-copy send, it will fall
    /// back to normal send.
    pub async fn send_zerocopy<T: IoBuf>(
        &self,
        buf: T,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        self.inner.send_zerocopy(buf, MSG_NOSIGNAL).await
    }

    /// Sends vectorized data using [zero-copy send](https://man7.org/linux/man-pages/man3/io_uring_prep_send_zc.3.html).
    ///
    /// If the underlying platform doesn't support zero-copy send, it will fall
    /// back to normal send.
    pub async fn send_zerocopy_vectored<T: IoVectoredBuf>(
        &self,
        buf: T,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        self.inner.send_zerocopy_vectored(buf, MSG_NOSIGNAL).await
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
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        (&*self).read_managed(len).await
    }
}

impl AsyncReadManaged for &TcpStream {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        self.inner.recv_managed(len, 0).await
    }
}

impl AsyncReadMulti for TcpStream {
    fn read_multi(&mut self, len: usize) -> impl Stream<Item = io::Result<Self::Buffer>> {
        self.inner.recv_multi(len, 0)
    }
}

impl AsyncReadMulti for &TcpStream {
    fn read_multi(&mut self, len: usize) -> impl Stream<Item = io::Result<Self::Buffer>> {
        self.inner.recv_multi(len, 0)
    }
}

impl AsyncReadAncillary for TcpStream {
    #[inline]
    async fn read_with_ancillary<T: IoBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        (&*self).read_with_ancillary(buffer, control).await
    }

    #[inline]
    async fn read_vectored_with_ancillary<T: IoVectoredBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        (&*self).read_vectored_with_ancillary(buffer, control).await
    }
}

impl AsyncReadAncillary for &TcpStream {
    #[inline]
    async fn read_with_ancillary<T: IoBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        self.inner
            .recv_msg(buffer, control, 0)
            .await
            .map_res(|(res, len, _addr)| (res, len))
    }

    #[inline]
    async fn read_vectored_with_ancillary<T: IoVectoredBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        self.inner
            .recv_msg_vectored(buffer, control, 0)
            .await
            .map_res(|(res, len, _addr)| (res, len))
    }
}

impl AsyncReadAncillaryManaged for TcpStream {
    #[inline]
    async fn read_managed_with_ancillary<C: IoBufMut>(
        &mut self,
        len: usize,
        control: C,
    ) -> io::Result<Option<(Self::Buffer, C)>> {
        (&*self).read_managed_with_ancillary(len, control).await
    }
}

impl AsyncReadAncillaryManaged for &TcpStream {
    #[inline]
    async fn read_managed_with_ancillary<C: IoBufMut>(
        &mut self,
        len: usize,
        control: C,
    ) -> io::Result<Option<(Self::Buffer, C)>> {
        self.inner
            .recv_msg_managed(len, control, 0)
            .await
            .map(|res| res.map(|(res, len, _addr)| (res, len)))
    }
}

impl AsyncReadAncillaryMulti for TcpStream {
    type Return = RecvMsgMultiResult;

    #[inline]
    fn read_multi_with_ancillary(
        &mut self,
        control_len: usize,
    ) -> impl Stream<Item = io::Result<Self::Return>> {
        self.inner.recv_msg_multi(control_len, 0)
    }
}

impl AsyncReadAncillaryMulti for &TcpStream {
    type Return = RecvMsgMultiResult;

    #[inline]
    fn read_multi_with_ancillary(
        &mut self,
        control_len: usize,
    ) -> impl Stream<Item = io::Result<Self::Return>> {
        self.inner.recv_msg_multi(control_len, 0)
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
        self.inner.send(buf, MSG_NOSIGNAL).await
    }

    #[inline]
    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.inner.send_vectored(buf, MSG_NOSIGNAL).await
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

impl AsyncWriteAncillary for TcpStream {
    #[inline]
    async fn write_with_ancillary<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        (&*self).write_with_ancillary(buffer, control).await
    }

    #[inline]
    async fn write_vectored_with_ancillary<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        (&*self)
            .write_vectored_with_ancillary(buffer, control)
            .await
    }
}

impl AsyncWriteAncillary for &TcpStream {
    #[inline]
    async fn write_with_ancillary<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        self.inner
            .send_msg(buffer, control, None, MSG_NOSIGNAL)
            .await
    }

    #[inline]
    async fn write_vectored_with_ancillary<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        self.inner
            .send_msg_vectored(buffer, control, None, MSG_NOSIGNAL)
            .await
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

impl<'a> Splittable for &'a mut TcpStream {
    type ReadHalf = ReadHalf<'a, TcpStream>;
    type WriteHalf = WriteHalf<'a, TcpStream>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        crate::split(self)
    }
}

impl_raw_fd!(TcpStream, socket2::Socket, inner, socket);

/// A TCP socket that has not yet been converted to a [`TcpStream`] or
/// [`TcpListener`].
#[derive(Debug)]
pub struct TcpSocket {
    inner: Socket,
}

impl TcpSocket {
    /// Creates a new socket configured for IPv4.
    pub async fn new_v4() -> io::Result<TcpSocket> {
        TcpSocket::new(socket2::Domain::IPV4).await
    }

    /// Creates a new socket configured for IPv6.
    pub async fn new_v6() -> io::Result<TcpSocket> {
        TcpSocket::new(socket2::Domain::IPV6).await
    }

    async fn new(domain: socket2::Domain) -> io::Result<TcpSocket> {
        let inner =
            Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP)).await?;
        Ok(TcpSocket { inner })
    }

    /// Sets value for the `SO_KEEPALIVE` option on this socket.
    pub fn set_keepalive(&self, keepalive: bool) -> io::Result<()> {
        self.inner.socket.set_keepalive(keepalive)
    }

    /// Gets the value of the `SO_KEEPALIVE` option on this socket.
    pub fn keepalive(&self) -> io::Result<bool> {
        self.inner.socket.keepalive()
    }

    /// Allows the socket to bind to an in-use address.
    pub fn set_reuseaddr(&self, reuseaddr: bool) -> io::Result<()> {
        self.inner.socket.set_reuse_address(reuseaddr)
    }

    /// Retrieves the value set for `SO_REUSEADDR` on this socket.
    pub fn reuseaddr(&self) -> io::Result<bool> {
        self.inner.socket.reuse_address()
    }

    /// Allows the socket to bind to an in-use port. Only available for
    /// supported unix systems.
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    pub fn set_reuseport(&self, reuseport: bool) -> io::Result<()> {
        self.inner.socket.set_reuse_port(reuseport)
    }

    /// Allows the socket to bind to an in-use port. Only available for
    /// supported unix systems.
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    pub fn reuseport(&self) -> io::Result<bool> {
        self.inner.socket.reuse_port()
    }

    /// Sets the size of the TCP send buffer on this socket.
    ///
    /// On most operating systems, this sets the `SO_SNDBUF` socket option.
    pub fn set_send_buffer_size(&self, size: u32) -> io::Result<()> {
        self.inner.socket.set_send_buffer_size(size as usize)
    }

    /// Returns the size of the TCP send buffer for this socket.
    ///
    /// On most operating systems, this is the value of the `SO_SNDBUF` socket
    /// option.
    pub fn send_buffer_size(&self) -> io::Result<u32> {
        self.inner.socket.send_buffer_size().map(|n| n as u32)
    }

    /// Sets the size of the TCP receive buffer on this socket.
    ///
    /// On most operating systems, this sets the `SO_RCVBUF` socket option.
    pub fn set_recv_buffer_size(&self, size: u32) -> io::Result<()> {
        self.inner.socket.set_recv_buffer_size(size as usize)
    }

    /// Returns the size of the TCP receive buffer for this socket.
    ///
    /// On most operating systems, this is the value of the `SO_RCVBUF` socket
    /// option.
    pub fn recv_buffer_size(&self) -> io::Result<u32> {
        self.inner.socket.recv_buffer_size().map(|n| n as u32)
    }

    /// Sets a linger duration of zero on this socket by setting the `SO_LINGER`
    /// option.
    pub fn set_zero_linger(&self) -> io::Result<()> {
        self.inner.socket.set_linger(Some(Duration::ZERO))
    }

    /// Reads the linger duration for this socket by getting the `SO_LINGER`
    /// option.
    pub fn linger(&self) -> io::Result<Option<Duration>> {
        self.inner.socket.linger()
    }

    /// Sets the value of the `TCP_NODELAY` option on this socket.
    ///
    /// If set, this option disables the Nagle algorithm. This means that
    /// segments are always sent as soon as possible, even if there is only
    /// a small amount of data. When not set, data is buffered until there
    /// is a sufficient amount to send out, thereby avoiding the frequent
    /// sending of small packets.
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.inner.socket.set_tcp_nodelay(nodelay)
    }

    /// Gets the value of the `TCP_NODELAY` option on this socket.
    ///
    /// For more information about this option, see [`set_nodelay`].
    ///
    /// [`set_nodelay`]: TcpSocket::set_nodelay
    pub fn nodelay(&self) -> io::Result<bool> {
        self.inner.socket.tcp_nodelay()
    }

    /// Gets the value of the `IPV6_TCLASS` option for this socket.
    ///
    /// For more information about this option, see [`set_tclass_v6`].
    ///
    /// [`set_tclass_v6`]: Self::set_tclass_v6
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "cygwin",
    ))]
    pub fn tclass_v6(&self) -> io::Result<u32> {
        self.inner.socket.tclass_v6()
    }

    /// Sets the value for the `IPV6_TCLASS` option on this socket.
    ///
    /// Specifies the traffic class field that is used in every packet
    /// sent from this socket.
    ///
    /// # Note
    ///
    /// This may not have any effect on IPv4 sockets.
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "cygwin",
    ))]
    pub fn set_tclass_v6(&self, tclass: u32) -> io::Result<()> {
        self.inner.socket.set_tclass_v6(tclass)
    }

    /// Gets the value of the `IP_TOS` option for this socket.
    ///
    /// For more information about this option, see [`set_tos_v4`].
    ///
    /// [`set_tos_v4`]: Self::set_tos_v4
    #[cfg(not(any(
        target_os = "fuchsia",
        target_os = "redox",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku"
    )))]
    pub fn tos_v4(&self) -> io::Result<u32> {
        self.inner.socket.tos_v4()
    }

    /// Sets the value for the `IP_TOS` option on this socket.
    ///
    /// This value sets the type-of-service field that is used in every packet
    /// sent from this socket.
    ///
    /// # Note
    ///
    /// - This may not have any effect on IPv6 sockets.
    /// - On Windows, `IP_TOS` is only supported on [Windows 8+ or
    ///   Windows Server 2012+.](https://docs.microsoft.com/en-us/windows/win32/winsock/ipproto-ip-socket-options)
    #[cfg(not(any(
        target_os = "fuchsia",
        target_os = "redox",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku"
    )))]
    pub fn set_tos_v4(&self, tos: u32) -> io::Result<()> {
        self.inner.socket.set_tos_v4(tos)
    }

    /// Gets the value for the `SO_BINDTODEVICE` option on this socket
    ///
    /// Returns the interface name of the device to which this socket is bound.
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux",))]
    pub fn device(&self) -> io::Result<Option<Vec<u8>>> {
        self.inner.socket.device()
    }

    /// Sets the value for the `SO_BINDTODEVICE` option on this socket
    ///
    /// If a socket is bound to an interface, only packets received from that
    /// particular interface are processed by the socket. Note that this only
    /// works for some socket types, particularly `AF_INET` sockets.
    ///
    /// If `interface` is `None` or an empty string it removes the binding.
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    pub fn bind_device(&self, interface: Option<&[u8]>) -> io::Result<()> {
        self.inner.socket.bind_device(interface)
    }

    /// Gets the local address of this socket.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self
            .inner
            .local_addr()?
            .as_socket()
            .expect("should be SocketAddr"))
    }

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
    }

    /// Binds the socket to the given address.
    pub async fn bind(&self, addr: SocketAddr) -> io::Result<()> {
        self.inner.bind(&addr.into()).await
    }

    /// Establishes a TCP connection with a peer at the specified socket
    /// address.
    ///
    /// The [`TcpSocket`] is consumed. Once the connection is established, a
    /// connected [`TcpStream`] is returned. If the connection fails, the
    /// encountered error is returned.
    pub async fn connect(self, addr: SocketAddr) -> io::Result<TcpStream> {
        self.inner.connect_async(&addr.into()).await?;
        Ok(TcpStream { inner: self.inner })
    }

    /// Converts the socket into a `TcpListener`.
    ///
    /// `backlog` defines the maximum number of pending connections that are
    /// queued by the operating system at any given time. Connections are
    /// removed from the queue with [`TcpListener::accept`]. When the queue
    /// is full, the operating system will start rejecting connections.
    pub async fn listen(self, backlog: i32) -> io::Result<TcpListener> {
        self.inner.listen(backlog).await?;
        Ok(TcpListener { inner: self.inner })
    }

    /// Converts a [`std::net::TcpStream`] into a [`TcpSocket`]. The provided
    /// socket must not have been connected prior to calling this function. This
    /// function is typically used together with crates such as [`socket2`] to
    /// configure socket options that are not available on [`TcpSocket`].
    pub fn from_std_stream(stream: std::net::TcpStream) -> io::Result<TcpSocket> {
        Ok(Self {
            inner: Socket::from_socket2(Socket2::from(stream))?,
        })
    }
}

impl_raw_fd!(TcpSocket, socket2::Socket, inner, socket);
