cfg_select! {
    windows => {
        #[path = "windows.rs"]
        mod sys;
    }
    unix => {
        #[path = "unix.rs"]
        mod sys;
    }
    _ => {}
}

#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::{
    future::poll_fn,
    io,
    net::Shutdown,
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::IntoInner;
use compio_driver::{AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd};
use socket2::{SockAddr, Socket};

/// Providing functionalities to wait for readiness.
///
/// ## Platform specific
/// * Windows: only supports sockets.
#[derive(Debug)]
pub struct PollFd<T: AsFd>(sys::PollFd<T>);

impl<T: AsFd> PollFd<T> {
    fn with_socket<R>(&self, f: impl FnOnce(&Socket) -> io::Result<R>) -> io::Result<R> {
        sys::with_socket(self.0.as_fd(), f)
    }

    /// Create [`PollFd`] without attaching the source.
    ///
    /// Ready-based sources does not need to be attached.
    pub fn new(source: T) -> io::Result<Self> {
        Self::from_shared_fd(SharedFd::new(source))
    }

    /// Create [`PollFd`] from a shared file descriptor.
    pub fn from_shared_fd(inner: SharedFd<T>) -> io::Result<Self> {
        sys::with_socket(inner.as_fd(), |socket| socket.set_nonblocking(true))?;
        Ok(Self(sys::PollFd::new(inner)?))
    }
}

impl<T: AsFd + 'static> PollFd<T> {
    /// Accept a connection from this socket.
    pub async fn accept(&self) -> io::Result<(PollFd<Socket>, SockAddr)> {
        poll_fn(|cx| self.poll_accept(cx)).await
    }

    /// Poll to accept a connection from this socket.
    pub fn poll_accept(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<(PollFd<Socket>, SockAddr)>> {
        self.poll_accept_with(cx, |source| {
            let (socket, addr) = sys::with_socket(source.as_fd(), Socket::accept)?;
            Ok((PollFd::new(socket)?, addr))
        })
    }

    /// Connect this socket to the specified address.
    pub async fn connect(&self, addr: &SockAddr) -> io::Result<()> {
        match self.with_socket(|socket| socket.connect(addr)) {
            Ok(()) => return Ok(()),
            Err(e) if is_connect_pending(&e) => {}
            Err(e) => return Err(e),
        }

        self.connect_ready().await?;
        self.with_socket(|socket| match socket.take_error()? {
            Some(e) => Err(e),
            None => Ok(()),
        })
    }

    /// Wait for accept readiness, before calling `accept`, or after `accept`
    /// returns `WouldBlock`.
    pub async fn accept_ready(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_accept_ready(cx)).await
    }

    /// Wait for connect readiness.
    pub async fn connect_ready(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_connect_ready(cx)).await
    }

    /// Wait for read readiness.
    pub async fn read_ready(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_read_ready(cx)).await
    }

    /// Wait for write readiness.
    pub async fn write_ready(&self) -> io::Result<()> {
        poll_fn(|cx| self.poll_write_ready(cx)).await
    }

    /// Poll for accept readiness.
    pub fn poll_accept_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.poll_accept_ready(cx)
    }

    /// Poll for connect readiness.
    pub fn poll_connect_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.poll_connect_ready(cx)
    }

    /// Poll for read readiness.
    pub fn poll_read_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.poll_read_ready(cx)
    }

    /// Poll for write readiness.
    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.poll_write_ready(cx)
    }

    /// Poll for accept readiness and call the provided function.
    pub fn poll_accept_with<R>(
        &self,
        cx: &mut Context,
        mut f: impl FnMut(&T) -> io::Result<R>,
    ) -> Poll<io::Result<R>> {
        loop {
            match f(&self.0) {
                Ok(result) => break Poll::Ready(Ok(result)),
                Err(e) if is_would_block(&e) => {
                    std::task::ready!(self.poll_accept_ready(cx))?;
                }
                Err(e) => break Poll::Ready(Err(e)),
            }
        }
    }

    /// Poll for read readiness and call the provided function.
    pub fn poll_read_with<R>(
        &self,
        cx: &mut Context,
        mut f: impl FnMut(&T) -> io::Result<R>,
    ) -> Poll<io::Result<R>> {
        loop {
            match f(&self.0) {
                Ok(result) => break Poll::Ready(Ok(result)),
                Err(e) if is_would_block(&e) => {
                    std::task::ready!(self.poll_read_ready(cx))?;
                }
                Err(e) => break Poll::Ready(Err(e)),
            }
        }
    }

    /// Poll for write readiness and call the provided function.
    pub fn poll_write_with<R>(
        &self,
        cx: &mut Context,
        mut f: impl FnMut(&T) -> io::Result<R>,
    ) -> Poll<io::Result<R>> {
        loop {
            match f(&self.0) {
                Ok(result) => break Poll::Ready(Ok(result)),
                Err(e) if is_would_block(&e) => {
                    std::task::ready!(self.poll_write_ready(cx))?;
                }
                Err(e) => break Poll::Ready(Err(e)),
            }
        }
    }
}

impl<T: AsFd + 'static> PollFd<T>
where
    for<'a> &'a T: std::io::Read,
{
    /// Poll for read readiness and read data.
    pub fn poll_read(&self, cx: &mut Context, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.poll_read_with(cx, |fd| std::io::Read::read(&mut &*fd, buf))
    }

    /// Poll for read readiness and read data into an uninitialized buffer.
    #[cfg(feature = "read_buf")]
    pub fn poll_read_buf(
        &self,
        cx: &mut Context,
        mut buf: std::io::BorrowedCursor<u8>,
    ) -> Poll<io::Result<()>> {
        self.poll_read_with(cx, |fd| std::io::Read::read_buf(&mut &*fd, buf.reborrow()))
    }
}

impl<T: AsFd + 'static> PollFd<T>
where
    for<'a> &'a T: std::io::Write,
{
    /// Poll for write readiness and write data.
    pub fn poll_write(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.poll_write_with(cx, |fd| std::io::Write::write(&mut &*fd, buf))
    }

    /// Poll for write readiness and write data from a slice of buffers.
    ///
    /// Whether this is more efficient than [`poll_write`] depends on the
    /// source: it is a single `writev` for sockets and files, while other
    /// sources may fall back to writing the first non-empty buffer.
    ///
    /// [`poll_write`]: Self::poll_write
    pub fn poll_write_vectored(
        &self,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.poll_write_with(cx, |fd| std::io::Write::write_vectored(&mut &*fd, bufs))
    }

    /// Poll for write readiness and flush the source.
    pub fn poll_flush(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_write_with(cx, |fd| std::io::Write::flush(&mut &*fd))
    }
}

impl<T: AsFd> PollFd<T> {
    /// Shut down the write half, so the peer of a connected socket observes the
    /// end of the stream while this side can still read.
    ///
    /// Sources that cannot be half-closed report success and are left as they
    /// are, since there is no write half to shut down. Shutting down twice is
    /// successful as well.
    ///
    /// Like the `shutdown` methods in `std`, this does not flush the source.
    fn shutdown_write(&self) -> io::Result<()> {
        match self.with_socket(|socket| socket.shutdown(Shutdown::Write)) {
            Err(e) if e.kind() == io::ErrorKind::NotConnected => Ok(()),
            result => result,
        }
    }
}

impl<T: AsFd> IntoInner for PollFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}

impl<T: AsFd> ToSharedFd<T> for PollFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.0.to_shared_fd()
    }
}

impl<T: AsFd> AsFd for PollFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl<T: AsFd> AsRawFd for PollFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: AsFd + AsRawSocket> AsRawSocket for PollFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.0.as_raw_socket()
    }
}

impl<T: AsFd> Deref for PollFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn is_would_block(e: &io::Error) -> bool {
    cfg_select! {
        unix => {
            e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(libc::EINPROGRESS)
        }
        windows => {
            use windows_sys::Win32::Networking::WinSock::WSAEINPROGRESS;
            e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(WSAEINPROGRESS)
        }
        _ => {
            e.kind() == io::ErrorKind::WouldBlock
        }
    }
}

fn is_connect_pending(e: &io::Error) -> bool {
    if is_would_block(e) {
        return true;
    }

    cfg_select! {
        unix => {
            e.raw_os_error() == Some(libc::EALREADY)
        }
        windows => {
            use windows_sys::Win32::Networking::WinSock::WSAEALREADY;
            e.raw_os_error() == Some(WSAEALREADY)
        }
        _ => {
            false
        }
    }
}

impl<T: AsFd + 'static> futures_util::AsyncRead for &PollFd<T>
where
    for<'a> &'a T: std::io::Read,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        (*self).poll_read(cx, buf)
    }
}

impl<T: AsFd + 'static> futures_util::AsyncRead for PollFd<T>
where
    for<'a> &'a T: std::io::Read,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        (*self).poll_read(cx, buf)
    }
}

impl<T: AsFd + 'static> futures_util::AsyncWrite for &PollFd<T>
where
    for<'a> &'a T: std::io::Write,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        (*self).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        (*self).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (*self).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Deliberately not flushing: the write readiness registration is shared
        // with any pending write, so flushing here would steal its waker.
        Poll::Ready(self.shutdown_write())
    }
}

impl<T: AsFd + 'static> futures_util::AsyncWrite for PollFd<T>
where
    for<'a> &'a T: std::io::Write,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        (*self).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        (*self).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (*self).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Deliberately not flushing: the write readiness registration is shared
        // with any pending write, so flushing here would steal its waker.
        Poll::Ready(self.shutdown_write())
    }
}
