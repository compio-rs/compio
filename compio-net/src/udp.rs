use std::{
    future::Future,
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::{
    BufferRef, impl_raw_fd,
    op::{RecvFlags, RecvFromMultiResult, RecvMsgMultiResult},
};
use futures_util::Stream;
use socket2::{Protocol, SockAddr, Socket as Socket2, Type};

use crate::{MSG_NOSIGNAL, Socket, ToSocketAddrsAsync};

/// A UDP socket.
///
/// UDP is "connectionless", unlike TCP. Meaning, regardless of what address
/// you've bound to, a `UdpSocket` is free to communicate with many different
/// remotes. There are basically two main ways to use `UdpSocket`:
///
/// * one to many: [`bind`](`UdpSocket::bind`) and use
///   [`send_to`](`UdpSocket::send_to`) and
///   [`recv_from`](`UdpSocket::recv_from`) to communicate with many different
///   addresses
/// * one to one: [`connect`](`UdpSocket::connect`) and associate with a single
///   address, using [`send`](`UdpSocket::send`) and [`recv`](`UdpSocket::recv`)
///   to communicate only with that remote address
///
/// # Examples
/// Bind and connect a pair of sockets and send a packet:
///
/// ```
/// use std::net::SocketAddr;
///
/// use compio_net::UdpSocket;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// let first_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
/// let second_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///
/// // bind sockets
/// let mut socket = UdpSocket::bind(first_addr).await.unwrap();
/// let first_addr = socket.local_addr().unwrap();
/// let mut other_socket = UdpSocket::bind(second_addr).await.unwrap();
/// let second_addr = other_socket.local_addr().unwrap();
///
/// // connect sockets
/// socket.connect(second_addr).await.unwrap();
/// other_socket.connect(first_addr).await.unwrap();
///
/// let buf = Vec::with_capacity(12);
///
/// // write data
/// socket.send("Hello world!").await.unwrap();
///
/// // read data
/// let (n_bytes, buf) = other_socket.recv(buf).await.unwrap();
///
/// assert_eq!(n_bytes, buf.len());
/// assert_eq!(buf, b"Hello world!");
/// # });
/// ```
/// Send and receive packets without connecting:
///
/// ```
/// use std::net::SocketAddr;
///
/// use compio_net::UdpSocket;
/// use socket2::SockAddr;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// let first_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
/// let second_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///
/// // bind sockets
/// let mut socket = UdpSocket::bind(first_addr).await.unwrap();
/// let first_addr = socket.local_addr().unwrap();
/// let mut other_socket = UdpSocket::bind(second_addr).await.unwrap();
/// let second_addr = other_socket.local_addr().unwrap();
///
/// let buf = Vec::with_capacity(32);
///
/// // write data
/// socket.send_to("hello world", second_addr).await.unwrap();
///
/// // read data
/// let ((n_bytes, addr), buf) = other_socket.recv_from(buf).await.unwrap();
///
/// assert_eq!(addr, first_addr);
/// assert_eq!(n_bytes, buf.len());
/// assert_eq!(buf, b"hello world");
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct UdpSocket {
    inner: Socket,
}

impl UdpSocket {
    /// Creates a new UDP socket and attempt to bind it to the addr provided.
    pub async fn bind(addr: impl ToSocketAddrsAsync) -> io::Result<Self> {
        super::each_addr(addr, |addr| async move {
            let addr = SockAddr::from(addr);
            let socket = Socket::new(addr.domain(), Type::DGRAM, Some(Protocol::UDP)).await?;
            socket.bind(&addr).await?;
            Ok(Self { inner: socket })
        })
        .await
    }

    /// Connects this UDP socket to a remote address, allowing the `send` and
    /// `recv` to be used to send data and also applies filters to only
    /// receive data from the specified address.
    ///
    /// Note that usually, a successful `connect` call does not specify
    /// that there is a remote server listening on the port, rather, such an
    /// error would only be detected after the first send.
    pub async fn connect(&self, addr: impl ToSocketAddrsAsync) -> io::Result<()> {
        super::each_addr(addr, |addr| async move {
            self.inner.connect(&SockAddr::from(addr))
        })
        .await
    }

    /// Creates new UdpSocket from a std::net::UdpSocket.
    pub fn from_std(socket: std::net::UdpSocket) -> io::Result<Self> {
        Ok(Self {
            inner: Socket::from_socket2(Socket2::from(socket))?,
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

    /// Returns the socket address of the remote peer this socket was connected
    /// to.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    ///
    /// use compio_net::UdpSocket;
    /// use socket2::SockAddr;
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async {
    /// let socket = UdpSocket::bind("127.0.0.1:34254")
    ///     .await
    ///     .expect("couldn't bind to address");
    /// socket
    ///     .connect("192.168.0.1:41203")
    ///     .await
    ///     .expect("couldn't connect to address");
    /// assert_eq!(
    ///     socket.peer_addr().unwrap(),
    ///     SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 168, 0, 1), 41203))
    /// );
    /// # });
    /// ```
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.inner
            .peer_addr()
            .map(|addr| addr.as_socket().expect("should be SocketAddr"))
    }

    /// Returns the local address that this socket is bound to.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::SocketAddr;
    ///
    /// use compio_net::UdpSocket;
    /// use socket2::SockAddr;
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async {
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// let sock = UdpSocket::bind(&addr).await.unwrap();
    /// // the address the socket is bound to
    /// let local_addr = sock.local_addr().unwrap();
    /// assert_eq!(local_addr, addr);
    /// # });
    /// ```
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner
            .local_addr()
            .map(|addr| addr.as_socket().expect("should be SocketAddr"))
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    pub async fn recv<T: IoBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv(buffer, RecvFlags::empty()).await
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    pub async fn recv_vectored<T: IoVectoredBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv_vectored(buffer, RecvFlags::empty()).await
    }

    /// Read some bytes from this source and return a [`BufferRef`].
    ///
    /// If `len` == 0, will use buffer pool's inner buffer size as the max len;
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len.
    pub async fn recv_managed(&self, len: usize) -> io::Result<Option<BufferRef>> {
        self.inner.recv_managed(len, RecvFlags::empty()).await
    }

    /// Read some bytes from this source and return a stream of [`BufferRef`]s.
    ///
    /// If `len` == 0, will use buffer pool's inner buffer size as the max len
    /// of each buffer; if `len` > 0, `min(len, inner buffer size)` will be
    /// the read max len of each buffer.
    pub fn recv_multi(&self, len: usize) -> impl Stream<Item = io::Result<BufferRef>> {
        self.inner.recv_multi(len, RecvFlags::empty())
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    pub async fn send<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send(buffer, MSG_NOSIGNAL).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    pub async fn send_vectored<T: IoVectoredBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send_vectored(buffer, MSG_NOSIGNAL).await
    }

    /// Receives a single datagram message on the socket. On success, returns
    /// the number of bytes received and the origin.
    pub async fn recv_from<T: IoBufMut>(&self, buffer: T) -> BufResult<(usize, SocketAddr), T> {
        self.inner
            .recv_from(buffer, RecvFlags::empty())
            .await
            .map_res(|(n, addr)| {
                let addr = addr
                    .expect("should have addr")
                    .as_socket()
                    .expect("should be SocketAddr");
                (n, addr)
            })
    }

    /// Receives a single datagram message on the socket. On success, returns
    /// the number of bytes received and the origin.
    pub async fn recv_from_vectored<T: IoVectoredBufMut>(
        &self,
        buffer: T,
    ) -> BufResult<(usize, SocketAddr), T> {
        self.inner
            .recv_from_vectored(buffer, RecvFlags::empty())
            .await
            .map_res(|(n, addr)| {
                let addr = addr
                    .expect("should have addr")
                    .as_socket()
                    .expect("should be SocketAddr");
                (n, addr)
            })
    }

    /// Read some bytes from this source and the runtime's buffer pool and
    /// return a [`BufferRef`] with the sender address.
    ///
    /// If `len` == 0, will use buffer pool's inner buffer size as the max len;
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    pub async fn recv_from_managed(
        &self,
        len: usize,
    ) -> io::Result<Option<(BufferRef, SocketAddr)>> {
        let res = self
            .inner
            .recv_from_managed(len, RecvFlags::empty())
            .await?;
        let ret = match res {
            Some((buffer, addr)) => {
                let addr = addr
                    .expect("should have addr")
                    .as_socket()
                    .expect("should be SocketAddr");
                Some((buffer, addr))
            }
            None => None,
        };
        Ok(ret)
    }

    /// Read some bytes from this source and the runtime's buffer pool and
    /// return a stream of [`RecvFromMultiResult`].
    pub fn recv_from_multi(&self) -> impl Stream<Item = io::Result<RecvFromMultiResult>> {
        self.inner.recv_from_multi(RecvFlags::empty())
    }

    /// Receives a single datagram message and ancillary data on the socket. On
    /// success, returns the number of bytes received and the origin.
    pub async fn recv_msg<T: IoBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize, SocketAddr), (T, C)> {
        self.inner
            .recv_msg(buffer, control, RecvFlags::empty())
            .await
            .map_res(|(n, m, addr)| {
                let addr = addr
                    .expect("should have addr")
                    .as_socket()
                    .expect("should be SocketAddr");
                (n, m, addr)
            })
    }

    /// Receives a single datagram message and ancillary data on the socket. On
    /// success, returns the number of bytes received and the origin.
    pub async fn recv_msg_vectored<T: IoVectoredBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize, SocketAddr), (T, C)> {
        self.inner
            .recv_msg_vectored(buffer, control, RecvFlags::empty())
            .await
            .map_res(|(n, m, addr)| {
                let addr = addr
                    .expect("should have addr")
                    .as_socket()
                    .expect("should be SocketAddr");
                (n, m, addr)
            })
    }

    /// Receives a single datagram message on the socket from the runtime's
    /// buffer pool, together with ancillary data. The ancillary data buffer is
    /// provided by the caller.
    ///
    /// If `len` == 0, will use buffer pool's inner buffer size as the max len;
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    pub async fn recv_msg_managed<C: IoBufMut>(
        &self,
        len: usize,
        control: C,
    ) -> io::Result<Option<(BufferRef, C, SocketAddr)>> {
        let res = self
            .inner
            .recv_msg_managed(len, control, RecvFlags::empty())
            .await?;
        let ret = match res {
            Some((buffer, control, addr)) => {
                let addr = addr
                    .expect("should have addr")
                    .as_socket()
                    .expect("should be SocketAddr");
                Some((buffer, control, addr))
            }
            None => None,
        };
        Ok(ret)
    }

    /// Receives multiple single datagram messages and ancillary data on the
    /// socket from the runtime's buffer pool.
    pub fn recv_msg_multi(
        &self,
        control_len: usize,
    ) -> impl Stream<Item = io::Result<RecvMsgMultiResult>> {
        self.inner.recv_msg_multi(control_len, RecvFlags::empty())
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes sent.
    pub async fn send_to<T: IoBuf>(
        &self,
        buffer: T,
        addr: impl ToSocketAddrsAsync,
    ) -> BufResult<usize, T> {
        super::first_addr_buf(addr, buffer, |addr, buffer| async move {
            self.inner
                .send_to(buffer, &SockAddr::from(addr), MSG_NOSIGNAL)
                .await
        })
        .await
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes sent.
    pub async fn send_to_vectored<T: IoVectoredBuf>(
        &self,
        buffer: T,
        addr: impl ToSocketAddrsAsync,
    ) -> BufResult<usize, T> {
        super::first_addr_buf(addr, buffer, |addr, buffer| async move {
            self.inner
                .send_to_vectored(buffer, &SockAddr::from(addr), MSG_NOSIGNAL)
                .await
        })
        .await
    }

    /// Sends data on the socket to the given address accompanied by ancillary
    /// data. On success, returns the number of bytes sent.
    pub async fn send_msg<T: IoBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: impl ToSocketAddrsAsync,
    ) -> BufResult<usize, (T, C)> {
        super::first_addr_buf(
            addr,
            (buffer, control),
            |addr, (buffer, control)| async move {
                self.inner
                    .send_msg(buffer, control, Some(&SockAddr::from(addr)), MSG_NOSIGNAL)
                    .await
            },
        )
        .await
    }

    /// Sends data on the socket to the given address accompanied by ancillary
    /// data. On success, returns the number of bytes sent.
    pub async fn send_msg_vectored<T: IoVectoredBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: impl ToSocketAddrsAsync,
    ) -> BufResult<usize, (T, C)> {
        super::first_addr_buf(
            addr,
            (buffer, control),
            |addr, (buffer, control)| async move {
                self.inner
                    .send_msg_vectored(buffer, control, Some(&SockAddr::from(addr)), MSG_NOSIGNAL)
                    .await
            },
        )
        .await
    }

    /// Sends data on the socket with zero copy.
    ///
    /// Returns the result of send and a future that resolves to the
    /// original buffer when the send is complete.
    pub async fn send_zerocopy<T: IoBuf>(
        &self,
        buf: T,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        self.inner.send_zerocopy(buf, MSG_NOSIGNAL).await
    }

    /// Sends vectored data on the socket with zero copy.
    ///
    /// Returns the result of send and a future that resolves to the
    /// original buffer when the send is complete.
    pub async fn send_zerocopy_vectored<T: IoVectoredBuf>(
        &self,
        buf: T,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        self.inner.send_zerocopy_vectored(buf, MSG_NOSIGNAL).await
    }

    /// Sends data on the socket to the given address with zero copy.
    ///
    /// Returns the result of send and a future that resolves to the
    /// original buffer when the send is complete.
    pub async fn send_to_zerocopy<A: ToSocketAddrsAsync, T: IoBuf>(
        &self,
        buffer: T,
        addr: A,
    ) -> BufResult<usize, impl Future<Output = T> + use<A, T>> {
        super::first_addr_buf_zerocopy(addr, buffer, |addr, buffer| async move {
            self.inner
                .send_to_zerocopy(buffer, &addr.into(), MSG_NOSIGNAL)
                .await
        })
        .await
    }

    /// Sends vectored data on the socket to the given address with zero copy.
    ///
    /// Returns the result of send and a future that resolves to the
    /// original buffer when the send is complete.
    pub async fn send_to_zerocopy_vectored<A: ToSocketAddrsAsync, T: IoVectoredBuf>(
        &self,
        buffer: T,
        addr: A,
    ) -> BufResult<usize, impl Future<Output = T> + use<A, T>> {
        super::first_addr_buf_zerocopy(addr, buffer, |addr, buffer| async move {
            self.inner
                .send_to_zerocopy_vectored(buffer, &addr.into(), MSG_NOSIGNAL)
                .await
        })
        .await
    }

    /// Sends data with control message on the socket to the given address with
    /// zero copy.
    ///
    /// Returns the result of send and a future that resolves to the
    /// original buffer when the send is complete.
    pub async fn send_msg_zerocopy<A: ToSocketAddrsAsync, T: IoBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: A,
    ) -> BufResult<usize, impl Future<Output = (T, C)> + use<A, T, C>> {
        super::first_addr_buf_zerocopy(addr, (buffer, control), |addr, (b, c)| async move {
            self.inner
                .send_msg_zerocopy(b, c, Some(&addr.into()), MSG_NOSIGNAL)
                .await
        })
        .await
    }

    /// Sends vectored data with control message on the socket to the given
    /// address with zero copy.
    ///
    /// Returns the result of send and a future that resolves to the
    /// original buffer when the send is complete.
    pub async fn send_msg_zerocopy_vectored<A: ToSocketAddrsAsync, T: IoVectoredBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: A,
    ) -> BufResult<usize, impl Future<Output = (T, C)> + use<A, T, C>> {
        super::first_addr_buf_zerocopy(addr, (buffer, control), |addr, (b, c)| async move {
            self.inner
                .send_msg_zerocopy_vectored(b, c, Some(&addr.into()), MSG_NOSIGNAL)
                .await
        })
        .await
    }

    /// Gets the value of the `SO_BROADCAST` option for this socket.
    ///
    /// For more information about this option, see [`set_broadcast`].
    ///
    /// [`set_broadcast`]: method@Self::set_broadcast
    pub fn broadcast(&self) -> io::Result<bool> {
        self.inner.socket.broadcast()
    }

    /// Sets the value of the `SO_BROADCAST` option for this socket.
    ///
    /// When enabled, this socket is allowed to send packets to a broadcast
    /// address.
    pub fn set_broadcast(&self, on: bool) -> io::Result<()> {
        self.inner.socket.set_broadcast(on)
    }

    /// Gets the value of the `IP_MULTICAST_LOOP` option for this socket.
    ///
    /// For more information about this option, see [`set_multicast_loop_v4`].
    ///
    /// [`set_multicast_loop_v4`]: method@Self::set_multicast_loop_v4
    pub fn multicast_loop_v4(&self) -> io::Result<bool> {
        self.inner.socket.multicast_loop_v4()
    }

    /// Sets the value of the `IP_MULTICAST_LOOP` option for this socket.
    ///
    /// If enabled, multicast packets will be looped back to the local socket.
    ///
    /// # Note
    ///
    /// This may not have any effect on IPv6 sockets.
    pub fn set_multicast_loop_v4(&self, on: bool) -> io::Result<()> {
        self.inner.socket.set_multicast_loop_v4(on)
    }

    /// Gets the value of the `IP_MULTICAST_TTL` option for this socket.
    ///
    /// For more information about this option, see [`set_multicast_ttl_v4`].
    ///
    /// [`set_multicast_ttl_v4`]: method@Self::set_multicast_ttl_v4
    pub fn multicast_ttl_v4(&self) -> io::Result<u32> {
        self.inner.socket.multicast_ttl_v4()
    }

    /// Sets the value of the `IP_MULTICAST_TTL` option for this socket.
    ///
    /// Indicates the time-to-live value of outgoing multicast packets for
    /// this socket. The default value is 1 which means that multicast packets
    /// don't leave the local network unless explicitly requested.
    ///
    /// # Note
    ///
    /// This may not have any effect on IPv6 sockets.
    pub fn set_multicast_ttl_v4(&self, ttl: u32) -> io::Result<()> {
        self.inner.socket.set_multicast_ttl_v4(ttl)
    }

    /// Gets the value of the `IPV6_MULTICAST_LOOP` option for this socket.
    ///
    /// For more information about this option, see [`set_multicast_loop_v6`].
    ///
    /// [`set_multicast_loop_v6`]: method@Self::set_multicast_loop_v6
    pub fn multicast_loop_v6(&self) -> io::Result<bool> {
        self.inner.socket.multicast_loop_v6()
    }

    /// Sets the value of the `IPV6_MULTICAST_LOOP` option for this socket.
    ///
    /// Controls whether this socket sees the multicast packets it sends itself.
    ///
    /// # Note
    ///
    /// This may not have any effect on IPv4 sockets.
    pub fn set_multicast_loop_v6(&self, on: bool) -> io::Result<()> {
        self.inner.socket.set_multicast_loop_v6(on)
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

    /// Gets the value of the `IP_TOS` option for this socket.
    ///
    /// For more information about this option, see [`set_tos_v4`].
    ///
    /// [`set_tos_v4`]: Self::set_tos_v4
    // https://docs.rs/socket2/0.6.1/src/socket2/socket.rs.html#1585
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
    /// This value gets the socket-bound device's interface name.
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

    /// Executes an operation of the `IP_ADD_MEMBERSHIP` type.
    ///
    /// This function specifies a new multicast group for this socket to join.
    /// The address must be a valid multicast address, and `interface` is the
    /// address of the local interface with which the system should join the
    /// multicast group. If it's equal to `INADDR_ANY` then an appropriate
    /// interface is chosen by the system.
    pub fn join_multicast_v4(&self, multiaddr: &Ipv4Addr, interface: &Ipv4Addr) -> io::Result<()> {
        self.inner.socket.join_multicast_v4(multiaddr, interface)
    }

    /// Executes an operation of the `IPV6_ADD_MEMBERSHIP` type.
    ///
    /// This function specifies a new multicast group for this socket to join.
    /// The address must be a valid multicast address, and `interface` is the
    /// index of the interface to join/leave (or 0 to indicate any interface).
    pub fn join_multicast_v6(&self, multiaddr: &Ipv6Addr, interface: u32) -> io::Result<()> {
        self.inner.socket.join_multicast_v6(multiaddr, interface)
    }

    /// Executes an operation of the `IP_DROP_MEMBERSHIP` type.
    ///
    /// For more information about this option, see [`join_multicast_v4`].
    ///
    /// [`join_multicast_v4`]: method@Self::join_multicast_v4
    pub fn leave_multicast_v4(&self, multiaddr: &Ipv4Addr, interface: &Ipv4Addr) -> io::Result<()> {
        self.inner.socket.leave_multicast_v4(multiaddr, interface)
    }

    /// Executes an operation of the `IPV6_DROP_MEMBERSHIP` type.
    ///
    /// For more information about this option, see [`join_multicast_v6`].
    ///
    /// [`join_multicast_v6`]: method@Self::join_multicast_v6
    pub fn leave_multicast_v6(&self, multiaddr: &Ipv6Addr, interface: u32) -> io::Result<()> {
        self.inner.socket.leave_multicast_v6(multiaddr, interface)
    }

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
    }

    /// Gets a socket option.
    ///
    /// # Safety
    ///
    /// The caller must ensure `T` is the correct type for `level` and `name`.
    pub unsafe fn get_socket_option<T: Copy>(&self, level: i32, name: i32) -> io::Result<T> {
        unsafe { self.inner.get_socket_option(level, name) }
    }

    /// Sets a socket option.
    ///
    /// # Safety
    ///
    /// The caller must ensure `T` is the correct type for `level` and `name`.
    pub unsafe fn set_socket_option<T: Copy>(
        &self,
        level: i32,
        name: i32,
        value: &T,
    ) -> io::Result<()> {
        unsafe { self.inner.set_socket_option(level, name, value) }
    }
}

impl_raw_fd!(UdpSocket, socket2::Socket, inner, socket);
