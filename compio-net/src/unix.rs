use std::{
    future::Future,
    io,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::{BufferRef, impl_raw_fd};
use compio_io::{
    AsyncRead, AsyncReadManaged, AsyncWrite,
    ancillary::{AsyncReadAncillary, AsyncWriteAncillary},
    util::Splittable,
};
use compio_runtime::fd::PollFd;
use futures_util::{Stream, StreamExt, stream::FusedStream};
use socket2::{Domain, SockAddr, Socket as Socket2, Type};

use crate::{Incoming, MSG_NOSIGNAL, OwnedReadHalf, OwnedWriteHalf, ReadHalf, Socket, WriteHalf};

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
    /// upon dropping [`UnixListener`].
    pub async fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::bind_addr(&SockAddr::unix(path)?).await
    }

    /// Creates a new [`UnixListener`] with [`SockAddr`], which will be bound to
    /// the specified file path. The file path cannot yet exist, and will be
    /// cleaned up upon dropping [`UnixListener`].
    pub async fn bind_addr(addr: &SockAddr) -> io::Result<Self> {
        if !addr.is_unix() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "addr is not unix socket address",
            ));
        }

        let socket = Socket::new(addr.domain(), Type::STREAM, None).await?;
        socket.bind(addr).await?;
        socket.listen(1024).await?;
        Ok(UnixListener { inner: socket })
    }

    #[cfg(unix)]
    /// Creates new UnixListener from a [`std::os::unix::net::UnixListener`].
    pub fn from_std(stream: std::os::unix::net::UnixListener) -> io::Result<Self> {
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
    /// This function will yield once a new Unix domain socket connection
    /// is established. When established, the corresponding [`UnixStream`] and
    /// will be returned.
    pub async fn accept(&self) -> io::Result<(UnixStream, SockAddr)> {
        let (socket, addr) = self.inner.accept().await?;
        let stream = UnixStream { inner: socket };
        Ok((stream, addr))
    }

    /// Returns a stream of incoming connections to this listener.
    pub fn incoming(&self) -> UnixIncoming<'_> {
        UnixIncoming {
            inner: self.inner.incoming(),
        }
    }

    /// Returns the local address that this listener is bound to.
    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.inner.local_addr()
    }

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
    }
}

impl_raw_fd!(UnixListener, socket2::Socket, inner, socket);

/// A stream of incoming Unix connections.
pub struct UnixIncoming<'a> {
    inner: Incoming<'a>,
}

impl Stream for UnixIncoming<'_> {
    type Item = io::Result<UnixStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.inner.poll_next_unpin(cx).map(|res| {
            res.map(|res| {
                let socket = res?;
                Ok(UnixStream { inner: socket })
            })
        })
    }
}

impl FusedStream for UnixIncoming<'_> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

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
    /// domain socket to successfully connect and return a [`UnixStream`].
    pub async fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::connect_addr(&SockAddr::unix(path)?).await
    }

    /// Opens a Unix connection to the specified address. There must be a
    /// [`UnixListener`] or equivalent listening on the corresponding Unix
    /// domain socket to successfully connect and return a [`UnixStream`].
    pub async fn connect_addr(addr: &SockAddr) -> io::Result<Self> {
        if !addr.is_unix() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "addr is not unix socket address",
            ));
        }
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None).await?;
        #[cfg(windows)]
        {
            let new_addr = empty_unix_socket();
            socket.bind(&new_addr).await?
        }
        socket.connect_async(addr).await?;
        let unix_stream = UnixStream { inner: socket };
        Ok(unix_stream)
    }

    #[cfg(unix)]
    /// Creates new UnixStream from a [`std::os::unix::net::UnixStream`].
    pub fn from_std(stream: std::os::unix::net::UnixStream) -> io::Result<Self> {
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

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
    }

    /// Splits a [`UnixStream`] into a read half and a write half, which can be
    /// used to read and write the stream concurrently.
    ///
    /// This method is more efficient than
    /// [`into_split`](UnixStream::into_split), but the halves cannot
    /// be moved into independently spawned tasks.
    pub fn split(&self) -> (ReadHalf<'_, Self>, WriteHalf<'_, Self>) {
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
        self.inner.recv(buf, 0).await
    }

    #[inline]
    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.inner.recv_vectored(buf, 0).await
    }
}

impl AsyncReadManaged for UnixStream {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        (&*self).read_managed(len).await
    }
}

impl AsyncReadManaged for &UnixStream {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        self.inner.recv_managed(len as _, 0).await
    }
}

impl AsyncReadAncillary for UnixStream {
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

impl AsyncReadAncillary for &UnixStream {
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

impl AsyncWriteAncillary for UnixStream {
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

impl AsyncWriteAncillary for &UnixStream {
    #[inline]
    async fn write_with_ancillary<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        self.inner.send_msg(buffer, control, None, 0).await
    }

    #[inline]
    async fn write_vectored_with_ancillary<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        self.inner.send_msg_vectored(buffer, control, None, 0).await
    }
}

impl Splittable for UnixStream {
    type ReadHalf = OwnedReadHalf<Self>;
    type WriteHalf = OwnedWriteHalf<Self>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        crate::into_split(self)
    }
}

impl<'a> Splittable for &'a UnixStream {
    type ReadHalf = ReadHalf<'a, UnixStream>;
    type WriteHalf = WriteHalf<'a, UnixStream>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        crate::split(self)
    }
}

impl<'a> Splittable for &'a mut UnixStream {
    type ReadHalf = ReadHalf<'a, UnixStream>;
    type WriteHalf = WriteHalf<'a, UnixStream>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        crate::split(self)
    }
}

impl_raw_fd!(UnixStream, socket2::Socket, inner, socket);

/// A Unix socket that has not yet been converted to a [`UnixStream`] or
/// [`UnixListener`].
#[derive(Debug)]
pub struct UnixSocket {
    inner: Socket,
}

impl UnixSocket {
    /// Creates a new Unix stream socket.
    pub async fn new_stream() -> io::Result<UnixSocket> {
        UnixSocket::new(socket2::Type::STREAM).await
    }

    async fn new(ty: socket2::Type) -> io::Result<UnixSocket> {
        let inner = Socket::new(socket2::Domain::UNIX, ty, None).await?;
        Ok(UnixSocket { inner })
    }

    /// Returns the local address that this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.inner.local_addr()
    }

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.inner.socket.take_error()
    }

    /// Binds the socket to the given address.
    pub async fn bind(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.bind_addr(&SockAddr::unix(path)?).await
    }

    /// Binds the socket to the given address.
    pub async fn bind_addr(&self, addr: &SockAddr) -> io::Result<()> {
        if !addr.is_unix() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "addr is not unix socket address",
            ));
        }
        self.inner.bind(addr).await
    }

    /// Converts the socket into a `UnixListener`.
    ///
    /// `backlog` defines the maximum number of pending connections are queued
    /// by the operating system at any given time. Connections are removed from
    /// the queue with [`UnixListener::accept`]. When the queue is full, the
    /// operating-system will start rejecting connections.
    pub async fn listen(self, backlog: i32) -> io::Result<UnixListener> {
        self.inner.listen(backlog).await?;
        Ok(UnixListener { inner: self.inner })
    }

    /// Establishes a Unix connection with a peer at the specified socket
    /// address.
    ///
    /// See [`UnixSocket::connect_addr`] for more details.
    pub async fn connect(self, path: impl AsRef<Path>) -> io::Result<UnixStream> {
        self.connect_addr(&SockAddr::unix(path)?).await
    }

    /// Establishes a Unix connection with a peer at the specified socket
    /// address.
    ///
    /// The [`UnixSocket`] is consumed. Once the connection is established, a
    /// connected [`UnixStream`] is returned. If the connection fails, the
    /// encountered error is returned.
    ///
    /// On Windows, the socket should be bound to an empty address before
    /// connecting.
    pub async fn connect_addr(self, addr: &SockAddr) -> io::Result<UnixStream> {
        if !addr.is_unix() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "addr is not unix socket address",
            ));
        }
        self.inner.connect_async(addr).await?;
        Ok(UnixStream { inner: self.inner })
    }
}

impl_raw_fd!(UnixSocket, socket2::Socket, inner, socket);

#[cfg(windows)]
#[inline]
fn empty_unix_socket() -> SockAddr {
    use windows_sys::Win32::Networking::WinSock::{AF_UNIX, SOCKADDR_UN};

    // SAFETY: the length is correct
    unsafe {
        SockAddr::try_init(|addr, len| {
            let addr: *mut SOCKADDR_UN = addr.cast();
            std::ptr::write(
                addr,
                SOCKADDR_UN {
                    sun_family: AF_UNIX,
                    sun_path: [0; 108],
                },
            );
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
    let sun_path = unsafe {
        std::slice::from_raw_parts(
            unix_addr.sun_path.as_ptr() as *const u8,
            unix_addr.sun_path.len(),
        )
    };
    let addr_len = match std::ffi::CStr::from_bytes_until_nul(sun_path) {
        Ok(str) => str.to_bytes_with_nul().len() + 2,
        Err(_) => std::mem::size_of::<SOCKADDR_UN>(),
    };
    unsafe {
        addr.set_length(addr_len as _);
    }
}
