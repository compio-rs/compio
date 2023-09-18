use std::io;

use socket2::{Protocol, SockAddr, Type};

#[cfg(feature = "runtime")]
use crate::{
    buf::{IoBuf, IoBufMut},
    BufResult,
};
use crate::{
    impl_raw_fd,
    net::{Socket, ToSockAddrs},
};

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
/// use compio::net::UdpSocket;
///
/// compio::task::block_on(async {
///     let first_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///     let second_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///
///     // bind sockets
///     let socket = UdpSocket::bind(first_addr).unwrap();
///     let first_addr = socket.local_addr().unwrap();
///     let other_socket = UdpSocket::bind(second_addr).unwrap();
///     let second_addr = other_socket.local_addr().unwrap();
///
///     // connect sockets
///     socket.connect(second_addr).unwrap();
///     other_socket.connect(first_addr).unwrap();
///
///     let buf = Vec::with_capacity(12);
///
///     // write data
///     let (result, _) = socket.send("Hello world!").await;
///     result.unwrap();
///
///     // read data
///     let (result, buf) = other_socket.recv(buf).await;
///     let n_bytes = result.unwrap();
///
///     assert_eq!(n_bytes, buf.len());
///     assert_eq!(buf, b"Hello world!");
/// });
/// ```
/// Send and receive packets without connecting:
///
/// ```
/// use std::net::SocketAddr;
///
/// use compio::net::UdpSocket;
/// use socket2::SockAddr;
///
/// compio::task::block_on(async {
///     let first_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///     let second_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///
///     // bind sockets
///     let socket = UdpSocket::bind(first_addr).unwrap();
///     let first_addr = socket.local_addr().unwrap();
///     let other_socket = UdpSocket::bind(second_addr).unwrap();
///     let second_addr = other_socket.local_addr().unwrap();
///
///     let buf = Vec::with_capacity(32);
///
///     // write data
///     let (result, _) = socket
///         .send_to("hello world", SockAddr::from(second_addr))
///         .await;
///     result.unwrap();
///
///     // read data
///     let (result, buf) = other_socket.recv_from(buf).await;
///     let (n_bytes, addr) = result.unwrap();
///
///     assert_eq!(addr, first_addr);
///     assert_eq!(n_bytes, buf.len());
///     assert_eq!(buf, b"hello world");
/// });
/// ```
pub struct UdpSocket {
    inner: Socket,
}

impl UdpSocket {
    /// Creates a new UDP socket and attempt to bind it to the addr provided.
    pub fn bind(addr: impl ToSockAddrs) -> io::Result<Self> {
        super::each_addr(addr, |addr| {
            Ok(Self {
                inner: Socket::bind(&addr, Type::DGRAM, Some(Protocol::UDP))?,
            })
        })
    }

    /// Connects this UDP socket to a remote address, allowing the `send` and
    /// `recv` to be used to send data and also applies filters to only
    /// receive data from the specified address.
    ///
    /// Note that usually, a successful `connect` call does not specify
    /// that there is a remote server listening on the port, rather, such an
    /// error would only be detected after the first send.
    pub fn connect(&self, addr: impl ToSockAddrs) -> io::Result<()> {
        super::each_addr(addr, |addr| self.inner.connect(&addr))
    }

    /// Creates a new independently owned handle to the underlying socket.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            inner: self.inner.try_clone()?,
        })
    }

    /// Returns the socket address of the remote peer this socket was connected
    /// to.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    ///
    /// use compio::net::UdpSocket;
    /// use socket2::SockAddr;
    ///
    /// let socket = UdpSocket::bind("127.0.0.1:34254").expect("couldn't bind to address");
    /// socket
    ///     .connect("192.168.0.1:41203")
    ///     .expect("couldn't connect to address");
    /// assert_eq!(
    ///     socket.peer_addr().unwrap(),
    ///     SockAddr::from(SocketAddr::V4(SocketAddrV4::new(
    ///         Ipv4Addr::new(192, 168, 0, 1),
    ///         41203
    ///     )))
    /// );
    /// ```
    pub fn peer_addr(&self) -> io::Result<SockAddr> {
        self.inner.peer_addr()
    }

    /// Returns the local address that this socket is bound to.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::SocketAddr;
    ///
    /// use compio::net::UdpSocket;
    /// use socket2::SockAddr;
    ///
    /// let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// let sock = UdpSocket::bind(&addr).unwrap();
    /// // the address the socket is bound to
    /// let local_addr = sock.local_addr().unwrap().as_socket().unwrap();
    /// assert_eq!(local_addr, addr);
    /// ```
    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.inner.local_addr()
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    #[cfg(feature = "runtime")]
    pub async fn recv<T: IoBufMut<'static>>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.recv(buffer).await
    }

    /// Receives a packet of data from the socket into the buffer, returning the
    /// original buffer and quantity of data received.
    #[cfg(feature = "runtime")]
    pub async fn recv_vectored<T: IoBufMut<'static>>(
        &self,
        buffer: Vec<T>,
    ) -> BufResult<usize, Vec<T>> {
        self.inner.recv_vectored(buffer).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    #[cfg(feature = "runtime")]
    pub async fn send<T: IoBuf<'static>>(&self, buffer: T) -> BufResult<usize, T> {
        self.inner.send(buffer).await
    }

    /// Sends some data to the socket from the buffer, returning the original
    /// buffer and quantity of data sent.
    #[cfg(feature = "runtime")]
    pub async fn send_vectored<T: IoBuf<'static>>(
        &self,
        buffer: Vec<T>,
    ) -> BufResult<usize, Vec<T>> {
        self.inner.send_vectored(buffer).await
    }

    /// Receives a single datagram message on the socket. On success, returns
    /// the number of bytes received and the origin.
    #[cfg(feature = "runtime")]
    pub async fn recv_from<T: IoBufMut<'static>>(
        &self,
        buffer: T,
    ) -> BufResult<(usize, SockAddr), T> {
        self.inner.recv_from(buffer).await
    }

    /// Receives a single datagram message on the socket. On success, returns
    /// the number of bytes received and the origin.
    #[cfg(feature = "runtime")]
    pub async fn recv_from_vectored<T: IoBufMut<'static>>(
        &self,
        buffer: Vec<T>,
    ) -> BufResult<(usize, SockAddr), Vec<T>> {
        self.inner.recv_from_vectored(buffer).await
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes sent.
    #[cfg(feature = "runtime")]
    pub async fn send_to<T: IoBuf<'static>>(
        &self,
        buffer: T,
        addr: impl ToSockAddrs,
    ) -> BufResult<usize, T> {
        super::each_addr_async_buf(addr, buffer, |addr, buffer| async move {
            self.inner.send_to(buffer, &addr).await
        })
        .await
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes sent.
    #[cfg(feature = "runtime")]
    pub async fn send_to_vectored<T: IoBuf<'static>>(
        &self,
        buffer: Vec<T>,
        addr: impl ToSockAddrs,
    ) -> BufResult<usize, Vec<T>> {
        super::each_addr_async_buf(addr, buffer, |addr, buffer| async move {
            self.inner.send_to_vectored(buffer, &addr).await
        })
        .await
    }
}

impl_raw_fd!(UdpSocket, inner);
