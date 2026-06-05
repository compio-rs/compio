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
    io,
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::IntoInner;
use compio_driver::{AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd};

/// Providing functionalities to wait for readiness.
#[derive(Debug)]
pub struct PollFd<T: AsFd>(sys::PollFd<T>);

impl<T: AsFd> PollFd<T> {
    /// Create [`PollFd`] without attaching the source.
    ///
    /// Ready-based sources does not need to be attached.
    pub fn new(source: T) -> io::Result<Self> {
        Self::from_shared_fd(SharedFd::new(source))
    }

    /// Create [`PollFd`] from a shared file descriptor.
    pub fn from_shared_fd(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self(sys::PollFd::new(inner)?))
    }
}

impl<T: AsFd + 'static> PollFd<T> {
    /// Wait for accept readiness, before calling `accept`, or after `accept`
    /// returns `WouldBlock`.
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.0.accept_ready().await
    }

    /// Wait for connect readiness.
    pub async fn connect_ready(&self) -> io::Result<()> {
        self.0.connect_ready().await
    }

    /// Wait for read readiness.
    pub async fn read_ready(&self) -> io::Result<()> {
        self.0.read_ready().await
    }

    /// Wait for write readiness.
    pub async fn write_ready(&self) -> io::Result<()> {
        self.0.write_ready().await
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

    /// Poll for connect readiness and call the provided function.
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
        mut buf: std::io::BorrowedCursor,
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
    #[cfg(unix)]
    {
        e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(libc::EINPROGRESS)
    }
    #[cfg(not(unix))]
    {
        e.kind() == io::ErrorKind::WouldBlock
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

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
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

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
