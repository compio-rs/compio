use std::{future::Future, io, net::SocketAddr};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::impl_raw_fd;
use compio_runtime::{BorrowedBuffer, BufferPool};
use socket2::{Protocol, SockAddr, Socket as Socket2, Type};

use crate::{Socket, SyncSocket, ToSocketAddrsAsync};

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
            Ok(Self {
                inner: Socket::bind(&SockAddr::from(addr), Type::DGRAM, Some(Protocol::UDP))
                    .await?,
            })
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
        self.inner.recv(buffer).await
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    pub async fn recv_vectored<T: IoVectoredBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv_vectored(buffer).await
    }

    /// Read some bytes from this source with [`BufferPool`] and return
    /// a [`BorrowedBuffer`].
    ///
    /// If `len` == 0, will use [`BufferPool`] inner buffer size as the max len,
    /// if `len` > 0, `min(len, inner buffer size)` will be the read max len
    pub async fn recv_managed<'a>(
        &self,
        buffer_pool: &'a BufferPool,
        len: usize,
    ) -> io::Result<BorrowedBuffer<'a>> {
        self.inner.recv_managed(buffer_pool, len).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    pub async fn send<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send(buffer).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    pub async fn send_vectored<T: IoVectoredBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send_vectored(buffer).await
    }

    /// Receives a single datagram message on the socket. On success, returns
    /// the number of bytes received and the origin.
    pub async fn recv_from<T: IoBufMut>(&self, buffer: T) -> BufResult<(usize, SocketAddr), T> {
        self.inner
            .recv_from(buffer)
            .await
            .map_res(|(n, addr)| (n, addr.as_socket().expect("should be SocketAddr")))
    }

    /// Receives a single datagram message on the socket. On success, returns
    /// the number of bytes received and the origin.
    pub async fn recv_from_vectored<T: IoVectoredBufMut>(
        &self,
        buffer: T,
    ) -> BufResult<(usize, SocketAddr), T> {
        self.inner
            .recv_from_vectored(buffer)
            .await
            .map_res(|(n, addr)| (n, addr.as_socket().expect("should be SocketAddr")))
    }

    /// Receives a single datagram message and ancillary data on the socket. On
    /// success, returns the number of bytes received and the origin.
    pub async fn recv_msg<T: IoBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize, SocketAddr), (T, C)> {
        self.inner
            .recv_msg(buffer, control)
            .await
            .map_res(|(n, m, addr)| (n, m, addr.as_socket().expect("should be SocketAddr")))
    }

    /// Receives a single datagram message and ancillary data on the socket. On
    /// success, returns the number of bytes received and the origin.
    pub async fn recv_msg_vectored<T: IoVectoredBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize, SocketAddr), (T, C)> {
        self.inner
            .recv_msg_vectored(buffer, control)
            .await
            .map_res(|(n, m, addr)| (n, m, addr.as_socket().expect("should be SocketAddr")))
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes sent.
    pub async fn send_to<T: IoBuf>(
        &self,
        buffer: T,
        addr: impl ToSocketAddrsAsync,
    ) -> BufResult<usize, T> {
        super::first_addr_buf(addr, buffer, |addr, buffer| async move {
            self.inner.send_to(buffer, &SockAddr::from(addr)).await
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
                .send_to_vectored(buffer, &SockAddr::from(addr))
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
                    .send_msg(buffer, control, &SockAddr::from(addr))
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
                    .send_msg_vectored(buffer, control, &SockAddr::from(addr))
                    .await
            },
        )
        .await
    }

    /// Gets a socket option.
    ///
    /// # Safety
    ///
    /// The caller must ensure `T` is the correct type for `level` and `name`.
    pub unsafe fn get_socket_option<T: Copy>(&self, level: i32, name: i32) -> io::Result<T> {
        self.inner.get_socket_option(level, name)
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
        self.inner.set_socket_option(level, name, value)
    }

    /// Attempts to clone the UDP socket.
    pub fn try_clone(&self) -> io::Result<SyncUdpSocket> {
        Ok(SyncUdpSocket::new(self.inner.try_clone()?))
    }

    /// Try to convert into a thread-safe UDP socket.
    pub fn try_into_sync(self) -> Result<SyncUdpSocket, Self> {
        match self.inner.try_into_sync() {
            Ok(sync_socket) => Ok(SyncUdpSocket::new(sync_socket)),
            Err(socket) => Err(UdpSocket { inner: socket }),
        }
    }
}

impl_raw_fd!(UdpSocket, socket2::Socket, inner, socket);

/// A thread-safe UDP socket.
#[derive(Debug)]
pub struct SyncUdpSocket {
    inner: SyncSocket,
}

impl SyncUdpSocket {
    pub(crate) fn new(inner: SyncSocket) -> Self {
        Self { inner }
    }

    /// Attempts to clone the UDP socket.
    pub fn try_clone(&self) -> io::Result<Self> {
        self.inner.try_clone().map(SyncUdpSocket::new)
    }
}

impl IntoInner for SyncUdpSocket {
    type Inner = UdpSocket;

    fn into_inner(self) -> Self::Inner {
        UdpSocket {
            inner: self.inner.into_inner(),
        }
    }
}
