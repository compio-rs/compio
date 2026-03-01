use std::{
    fmt::Debug,
    io::{self, BufRead},
    marker::PhantomPinned,
    mem::MaybeUninit,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{AsyncRead, AsyncWrite, PinBoxFuture, compat::SyncStream, util::DEFAULT_BUF_SIZE};

pin_project! {
    /// A stream wrapper for [`futures_util::io`] traits.
    pub struct AsyncStream<S> {
        #[pin]
        inner: SyncStream<S>,
        read_future: Option<PinBoxFuture<io::Result<usize>>>,
        write_future: Option<PinBoxFuture<io::Result<usize>>>,
        shutdown_future: Option<PinBoxFuture<io::Result<()>>>,
        #[pin]
        _p: PhantomPinned,
    }
}

impl<S> AsyncStream<S> {
    /// Create [`AsyncStream`] with the stream and default buffer size.
    pub fn new(stream: S) -> Self {
        Self::new_impl(SyncStream::new(stream))
    }

    /// Create [`AsyncStream`] with the stream and buffer size.
    pub fn with_capacity(cap: usize, stream: S) -> Self {
        Self::new_impl(SyncStream::with_capacity(cap, stream))
    }

    fn new_impl(inner: SyncStream<S>) -> Self {
        Self {
            inner,
            read_future: None,
            write_future: None,
            shutdown_future: None,
            _p: PhantomPinned,
        }
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut()
    }

    /// Consumes the `SyncStream`, returning the underlying stream.
    pub fn into_inner(self) -> S {
        self.inner.into_inner()
    }
}

pin_project! {
    /// A read stream wrapper for [`futures_util::io`].
    ///
    /// It doesn't support write and shutdown operations, making looser
    /// requirements on the inner stream.
    pub struct AsyncReadStream<S> {
        #[pin]
        inner: SyncStream<S>,
        read_future: Option<PinBoxFuture<io::Result<usize>>>,
        #[pin]
        _p: PhantomPinned,
    }
}

impl<S> AsyncReadStream<S> {
    /// Create [`AsyncReadStream`] with the stream and default buffer size.
    pub fn new(stream: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, stream)
    }

    /// Create [`AsyncReadStream`] with the stream and buffer size.
    pub fn with_capacity(cap: usize, stream: S) -> Self {
        Self::new_impl(SyncStream::with_limits2(
            cap,
            0,
            cap,
            SyncStream::<S>::DEFAULT_MAX_BUFFER,
            stream,
        ))
    }

    fn new_impl(inner: SyncStream<S>) -> Self {
        Self {
            inner,
            read_future: None,
            _p: PhantomPinned,
        }
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut()
    }

    /// Consumes the `SyncStream`, returning the underlying stream.
    pub fn into_inner(self) -> S {
        self.inner.into_inner()
    }
}

pin_project! {
    /// A write stream wrapper for [`futures_util::io`].
    ///
    /// It doesn't support read and shutdown operations, making looser requirements on the inner stream.
    pub struct AsyncWriteStream<S> {
        #[pin]
        inner: SyncStream<S>,
        write_future: Option<PinBoxFuture<io::Result<usize>>>,
        #[pin]
        _p: PhantomPinned,
    }
}

impl<S> AsyncWriteStream<S> {
    /// Create [`AsyncWriteStream`] with the stream and default buffer size.
    pub fn new(stream: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, stream)
    }

    /// Create [`AsyncWriteStream`] with the stream and buffer size.
    pub fn with_capacity(cap: usize, stream: S) -> Self {
        Self::new_impl(SyncStream::with_limits2(
            0,
            cap,
            cap,
            SyncStream::<S>::DEFAULT_MAX_BUFFER,
            stream,
        ))
    }

    fn new_impl(inner: SyncStream<S>) -> Self {
        Self {
            inner,
            write_future: None,
            _p: PhantomPinned,
        }
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut()
    }

    /// Consumes the `SyncStream`, returning the underlying stream.
    pub fn into_inner(self) -> S {
        self.inner.into_inner()
    }
}

macro_rules! poll_future {
    ($f:expr, $cx:expr, $e:expr) => {{
        let mut future = match $f.take() {
            Some(f) => f,
            None => Box::pin($e),
        };
        let f = future.as_mut();
        match f.poll($cx) {
            Poll::Pending => {
                $f.replace(future);
                return Poll::Pending;
            }
            Poll::Ready(res) => res,
        }
    }};
}

macro_rules! poll_future_would_block {
    ($f:expr, $cx:expr, $e:expr, $io:expr) => {{
        if let Some(mut f) = $f.take() {
            if f.as_mut().poll($cx).is_pending() {
                $f.replace(f);
                return Poll::Pending;
            }
        }

        match $io {
            Ok(len) => Poll::Ready(Ok(len)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                $f.replace(Box::pin($e));
                $cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }};
}

unsafe fn extend_lifetime_mut<T: ?Sized>(t: &mut T) -> &'static mut T {
    unsafe { &mut *(t as *mut T) }
}

unsafe fn extend_lifetime<T: ?Sized>(t: &T) -> &'static T {
    unsafe { &*(t as *const T) }
}

fn poll_read<S: AsyncRead + Unpin + 'static>(
    inner: &'static mut SyncStream<S>,
    cx: &mut Context<'_>,
    fut: &mut Option<PinBoxFuture<io::Result<usize>>>,
    buf: &mut [u8],
) -> Poll<io::Result<usize>> {
    poll_future_would_block!(fut, cx, inner.fill_read_buf(), io::Read::read(inner, buf))
}

fn poll_read_uninit<S: AsyncRead + Unpin + 'static>(
    inner: &'static mut SyncStream<S>,
    cx: &mut Context<'_>,
    fut: &mut Option<PinBoxFuture<io::Result<usize>>>,
    buf: &mut [MaybeUninit<u8>],
) -> Poll<io::Result<usize>> {
    poll_future_would_block!(fut, cx, inner.fill_read_buf(), inner.read_buf_uninit(buf))
}

fn poll_fill_buf<S: AsyncRead + Unpin + 'static>(
    inner: &'static mut SyncStream<S>,
    cx: &mut Context<'_>,
    fut: &mut Option<PinBoxFuture<io::Result<usize>>>,
) -> Poll<io::Result<&'static [u8]>> {
    poll_future_would_block!(
        fut,
        cx,
        inner.fill_read_buf(),
        // SAFETY: the future is none when ready.
        io::BufRead::fill_buf(inner).map(|slice| unsafe { extend_lifetime(slice) })
    )
}

impl<S: AsyncRead + Unpin + 'static> futures_util::AsyncRead for AsyncStream<S>
where
    for<'a> &'a S: AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`, and is internally mutable.
        // - The future only accesses the corresponding buffer and fields.
        //   - No access overlap between the futures.
        let inner = unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_read(inner, cx, this.read_future, buf)
    }
}

impl<S: AsyncRead + Unpin + 'static> futures_util::AsyncRead for AsyncReadStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`.
        let inner = unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_read(inner, cx, this.read_future, buf)
    }
}

impl<S: AsyncRead + Unpin + 'static> AsyncStream<S>
where
    for<'a> &'a S: AsyncRead,
{
    /// Attempt to read from the `AsyncRead` into `buf`.
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))`.
    pub fn poll_read_uninit(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_read_uninit(inner, cx, this.read_future, buf)
    }
}

impl<S: AsyncRead + Unpin + 'static> AsyncReadStream<S> {
    /// Attempt to read from the `AsyncRead` into `buf`.
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))`.
    pub fn poll_read_uninit(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_read_uninit(inner, cx, this.read_future, buf)
    }
}

impl<S: AsyncRead + Unpin + 'static> futures_util::AsyncBufRead for AsyncStream<S>
where
    for<'a> &'a S: AsyncRead,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_fill_buf(inner, cx, this.read_future)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        self.project().inner.consume(amt)
    }
}

impl<S: AsyncRead + Unpin + 'static> futures_util::AsyncBufRead for AsyncReadStream<S> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_fill_buf(inner, cx, this.read_future)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        self.project().inner.consume(amt)
    }
}

fn poll_write<S: AsyncWrite + Unpin + 'static>(
    inner: &'static mut SyncStream<S>,
    cx: &mut Context<'_>,
    fut: &mut Option<PinBoxFuture<io::Result<usize>>>,
    buf: &[u8],
) -> Poll<io::Result<usize>> {
    poll_future_would_block!(
        fut,
        cx,
        inner.flush_write_buf(),
        io::Write::write(inner, buf)
    )
}

fn poll_flush<S: AsyncWrite + Unpin + 'static>(
    inner: &'static mut SyncStream<S>,
    cx: &mut Context<'_>,
    fut: &mut Option<PinBoxFuture<io::Result<usize>>>,
) -> Poll<io::Result<()>> {
    poll_future!(fut, cx, inner.flush_write_buf())?;
    Poll::Ready(Ok(()))
}

impl<S: AsyncWrite + Unpin + 'static> futures_util::AsyncWrite for AsyncStream<S>
where
    for<'a> &'a S: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.shutdown_future.is_some() {
            debug_assert!(self.write_future.is_none());
            return Poll::Pending;
        }

        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_write(inner, cx, this.write_future, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.shutdown_future.is_some() {
            debug_assert!(self.write_future.is_none());
            return Poll::Pending;
        }

        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_flush(inner, cx, this.write_future)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Avoid shutdown on flush because the inner buffer might be passed to the
        // driver.
        if self.write_future.is_some() || self.inner.has_pending_write() {
            debug_assert!(self.shutdown_future.is_none());
            self.poll_flush(cx)
        } else {
            let this = self.project();
            let inner: &'static mut SyncStream<S> =
                unsafe { extend_lifetime_mut(this.inner.get_mut()) };
            let res = poll_future!(this.shutdown_future, cx, inner.get_mut().shutdown());
            Poll::Ready(res)
        }
    }
}

impl<S: AsyncWrite + Unpin + 'static> futures_util::AsyncWrite for AsyncWriteStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_write(inner, cx, this.write_future, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        poll_flush(inner, cx, this.write_future)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

impl<S: Debug> Debug for AsyncStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncStream")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod test {
    use futures_executor::block_on;
    use futures_util::AsyncWriteExt;

    use super::AsyncWriteStream;

    #[test]
    fn close() {
        block_on(async {
            let stream = AsyncWriteStream::new(Vec::<u8>::new());
            let mut stream = std::pin::pin!(stream);
            let n = stream.write(b"hello").await.unwrap();
            assert_eq!(n, 5);
            stream.close().await.unwrap();
            assert_eq!(stream.get_ref(), b"hello");
        })
    }
}
