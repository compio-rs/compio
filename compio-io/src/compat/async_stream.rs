use std::{
    fmt::Debug,
    io,
    marker::PhantomPinned,
    mem::MaybeUninit,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker, ready},
};

use pin_project_lite::pin_project;

use crate::{
    AsyncRead, AsyncWrite, PinBoxFuture,
    compat::{SyncStream, SyncStreamReadHalf, SyncStreamWriteHalf},
    util::{DEFAULT_BUF_SIZE, Splittable},
};

pin_project! {
    /// A stream wrapper for [`futures_util::io`] traits.
    pub struct AsyncStream<S: Splittable> {
        #[pin]
        read_inner: SyncStreamReadHalf<S::ReadHalf>,
        #[pin]
        write_inner: SyncStreamWriteHalf<S::WriteHalf>,
        read_future: Option<PinBoxFuture<io::Result<usize>>>,
        write_future: Option<PinBoxFuture<io::Result<usize>>>,
        shutdown_future: Option<PinBoxFuture<io::Result<()>>>,
        read_waker: Option<Waker>,
        read_uninit_waker: Option<Waker>,
        read_buf_waker: Option<Waker>,
        write_waker: Option<Waker>,
        flush_waker: Option<Waker>,
        close_waker: Option<Waker>,
        #[pin]
        _p: PhantomPinned,
    }
}

impl<S: Splittable> AsyncStream<S> {
    /// Create [`AsyncStream`] with the stream and default buffer size.
    pub fn new(stream: S) -> Self {
        Self::new_impl(SyncStream::new(stream))
    }

    /// Create [`AsyncStream`] with the stream and buffer size.
    pub fn with_capacity(cap: usize, stream: S) -> Self {
        Self::new_impl(SyncStream::with_capacity(cap, stream))
    }

    fn new_impl(inner: SyncStream<S>) -> Self {
        let (read_inner, write_inner) = inner.split();
        Self {
            read_inner,
            write_inner,
            read_future: None,
            write_future: None,
            shutdown_future: None,
            read_waker: None,
            read_uninit_waker: None,
            read_buf_waker: None,
            write_waker: None,
            flush_waker: None,
            close_waker: None,
            _p: PhantomPinned,
        }
    }
}

impl<S> AsyncStream<S>
where
    S: Splittable<ReadHalf = S, WriteHalf = S>,
{
    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        self.read_inner.get_ref()
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.read_inner.get_mut()
    }

    /// Consumes the `AsyncStream`, returning the underlying stream.
    pub fn into_inner(self) -> S {
        self.read_inner.into_inner()
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
        read_waker: Option<Waker>,
        read_uninit_waker: Option<Waker>,
        read_buf_waker: Option<Waker>,
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
            super::DEFAULT_MAX_BUFFER,
            stream,
        ))
    }

    fn new_impl(inner: SyncStream<S>) -> Self {
        Self {
            inner,
            read_future: None,
            read_waker: None,
            read_uninit_waker: None,
            read_buf_waker: None,
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
    /// It doesn't support read operations, making looser requirements on the inner stream.
    pub struct AsyncWriteStream<S> {
        #[pin]
        inner: SyncStream<S>,
        write_future: Option<PinBoxFuture<io::Result<usize>>>,
        shutdown_future: Option<PinBoxFuture<io::Result<()>>>,
        write_waker: Option<Waker>,
        flush_waker: Option<Waker>,
        close_waker: Option<Waker>,
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
            super::DEFAULT_MAX_BUFFER,
            stream,
        ))
    }

    fn new_impl(inner: SyncStream<S>) -> Self {
        Self {
            inner,
            write_future: None,
            shutdown_future: None,
            write_waker: None,
            flush_waker: None,
            close_waker: None,
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
    ($cx:expr, $w:expr, $io:expr, $f:expr) => {{
        match $io {
            Ok(res) => {
                $w.take();
                return Poll::Ready(Ok(res));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                ready!($f)?;
            }
            Err(e) => {
                $w.take();
                return Poll::Ready(Err(e));
            }
        }
    }};
}

unsafe fn extend_lifetime_mut<T: ?Sized>(t: &mut T) -> &'static mut T {
    unsafe { &mut *(t as *mut T) }
}

unsafe fn extend_lifetime<T: ?Sized>(t: &T) -> &'static T {
    unsafe { &*(t as *const T) }
}

fn replace_waker(waker_slot: &mut Option<Waker>, waker: &Waker) {
    if !waker_slot.as_ref().is_some_and(|w| w.will_wake(waker)) {
        waker_slot.replace(waker.clone());
    }
}

impl<S: Splittable + 'static> AsyncStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
{
    fn poll_read_impl(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is internally mutable.
        // - The future only accesses the corresponding buffer and fields.
        //   - No access overlap between the futures.
        let inner = unsafe { extend_lifetime_mut(this.read_inner.get_mut()) };
        let arr = WakerArray([
            this.read_waker.as_ref().cloned(),
            this.read_uninit_waker.as_ref().cloned(),
            this.read_buf_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.read_future, cx, inner.fill_read_buf());
        Poll::Ready(res)
    }
}

impl<S: Splittable + 'static> futures_util::AsyncRead for AsyncStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        replace_waker(self.as_mut().project().read_waker, cx.waker());
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.read_waker,
                io::Read::read(this.read_inner.get_mut(), buf),
                self.as_mut().poll_read_impl()
            )
        }
    }
}

impl<S: Splittable + 'static> AsyncStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
{
    /// Attempt to read from the `AsyncRead` into `buf`.
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))`.
    pub fn poll_read_uninit(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<io::Result<usize>> {
        replace_waker(self.as_mut().project().read_uninit_waker, cx.waker());
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.read_uninit_waker,
                this.read_inner.get_mut().read_buf_uninit(buf),
                self.as_mut().poll_read_impl()
            )
        }
    }
}

impl<S: Splittable + 'static> futures_util::AsyncBufRead for AsyncStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
{
    fn poll_fill_buf(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        replace_waker(self.as_mut().project().read_buf_waker, cx.waker());
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.read_buf_waker,
                // SAFETY: The buffer won't be accessed after the future is ready, and the future
                // won't live longer than the stream.
                io::BufRead::fill_buf(this.read_inner.get_mut())
                    .map(|s| unsafe { extend_lifetime(s) }),
                self.as_mut().poll_read_impl()
            )
        }
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        io::BufRead::consume(self.project().read_inner.get_mut(), amt)
    }
}

impl<S: AsyncRead + Unpin + 'static> AsyncReadStream<S> {
    fn poll_read_impl(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`.
        let inner = unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        let arr = WakerArray([
            this.read_waker.as_ref().cloned(),
            this.read_uninit_waker.as_ref().cloned(),
            this.read_buf_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.read_future, cx, inner.fill_read_buf());
        Poll::Ready(res)
    }
}

impl<S: AsyncRead + Unpin + 'static> futures_util::AsyncRead for AsyncReadStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        replace_waker(self.as_mut().project().read_waker, cx.waker());
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.read_waker,
                io::Read::read(this.inner.get_mut(), buf),
                self.as_mut().poll_read_impl()
            )
        }
    }
}

impl<S: AsyncRead + Unpin + 'static> AsyncReadStream<S> {
    /// Attempt to read from the `AsyncRead` into `buf`.
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))`.
    pub fn poll_read_uninit(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<io::Result<usize>> {
        replace_waker(self.as_mut().project().read_uninit_waker, cx.waker());
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.read_uninit_waker,
                this.inner.get_mut().read_buf_uninit(buf),
                self.as_mut().poll_read_impl()
            )
        }
    }
}
impl<S: AsyncRead + Unpin + 'static> futures_util::AsyncBufRead for AsyncReadStream<S> {
    fn poll_fill_buf(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        replace_waker(self.as_mut().project().read_buf_waker, cx.waker());
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.read_buf_waker,
                // SAFETY: The buffer won't be accessed after the future is ready, and the future
                // won't live longer than the stream.
                io::BufRead::fill_buf(this.inner.get_mut()).map(|s| unsafe { extend_lifetime(s) }),
                self.as_mut().poll_read_impl()
            )
        }
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        io::BufRead::consume(self.project().inner.get_mut(), amt)
    }
}

impl<S: Splittable + 'static> AsyncStream<S>
where
    S::WriteHalf: AsyncWrite + Unpin,
{
    fn poll_flush_impl(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is internally mutable.
        // - The future only accesses the corresponding buffer and fields.
        //   - No access overlap between the futures.
        let inner = unsafe { extend_lifetime_mut(this.write_inner.get_mut()) };
        let arr = WakerArray([
            this.write_waker.as_ref().cloned(),
            this.flush_waker.as_ref().cloned(),
            this.close_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.write_future, cx, inner.flush_write_buf());
        Poll::Ready(res)
    }

    fn poll_close_impl(self: Pin<&mut Self>) -> Poll<io::Result<()>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is internally mutable.
        // - The future only accesses the corresponding buffer and fields.
        //   - No access overlap between the futures.
        let inner = unsafe { extend_lifetime_mut(this.write_inner.get_mut()) };
        let arr = WakerArray([
            this.write_waker.as_ref().cloned(),
            this.flush_waker.as_ref().cloned(),
            this.close_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.shutdown_future, cx, inner.get_mut().shutdown());
        Poll::Ready(res)
    }
}

impl<S: Splittable + 'static> futures_util::AsyncWrite for AsyncStream<S>
where
    S::WriteHalf: AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        replace_waker(self.as_mut().project().write_waker, cx.waker());
        if self.shutdown_future.is_some() {
            debug_assert!(self.write_future.is_none());
            ready!(self.as_mut().poll_close_impl())?;
        }
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.write_waker,
                io::Write::write(this.write_inner.get_mut(), buf),
                self.as_mut().poll_flush_impl()
            )
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        replace_waker(self.as_mut().project().flush_waker, cx.waker());
        if self.shutdown_future.is_some() {
            debug_assert!(self.write_future.is_none());
            ready!(self.as_mut().poll_close_impl())?;
        }
        let res = ready!(self.as_mut().poll_flush_impl());
        self.project().flush_waker.take();
        Poll::Ready(res.map(|_| ()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        replace_waker(self.as_mut().project().close_waker, cx.waker());
        // Avoid shutdown on flush because the inner buffer might be passed to the
        // driver.
        if self.write_future.is_some() || self.write_inner.has_pending_write() {
            debug_assert!(self.shutdown_future.is_none());
            ready!(self.as_mut().poll_flush_impl())?;
        }
        let res = ready!(self.as_mut().poll_close_impl());
        self.project().close_waker.take();
        Poll::Ready(res)
    }
}

impl<S: AsyncWrite + Unpin + 'static> AsyncWriteStream<S> {
    fn poll_flush_impl(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`.
        let inner = unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        let arr = WakerArray([
            this.write_waker.as_ref().cloned(),
            this.flush_waker.as_ref().cloned(),
            this.close_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.write_future, cx, inner.flush_write_buf());
        Poll::Ready(res)
    }

    fn poll_close_impl(self: Pin<&mut Self>) -> Poll<io::Result<()>> {
        let this = self.project();
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`.
        let inner = unsafe { extend_lifetime_mut(this.inner.get_mut()) };
        let arr = WakerArray([
            this.write_waker.as_ref().cloned(),
            this.flush_waker.as_ref().cloned(),
            this.close_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.shutdown_future, cx, inner.get_mut().shutdown());
        Poll::Ready(res)
    }
}

impl<S: AsyncWrite + Unpin + 'static> futures_util::AsyncWrite for AsyncWriteStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        replace_waker(self.as_mut().project().write_waker, cx.waker());
        if self.shutdown_future.is_some() {
            debug_assert!(self.write_future.is_none());
            ready!(self.as_mut().poll_close_impl())?;
        }
        loop {
            let this = self.as_mut().project();
            poll_future_would_block!(
                cx,
                this.write_waker,
                io::Write::write(this.inner.get_mut(), buf),
                self.as_mut().poll_flush_impl()
            )
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        replace_waker(self.as_mut().project().flush_waker, cx.waker());
        if self.shutdown_future.is_some() {
            debug_assert!(self.write_future.is_none());
            ready!(self.as_mut().poll_close_impl())?;
        }
        let res = ready!(self.as_mut().poll_flush_impl());
        self.project().flush_waker.take();
        Poll::Ready(res.map(|_| ()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        replace_waker(self.as_mut().project().close_waker, cx.waker());
        // Avoid shutdown on flush because the inner buffer might be passed to the
        // driver.
        if self.write_future.is_some() || self.inner.has_pending_write() {
            debug_assert!(self.shutdown_future.is_none());
            ready!(self.as_mut().poll_flush_impl())?;
        }
        let res = ready!(self.as_mut().poll_close_impl());
        self.project().close_waker.take();
        Poll::Ready(res)
    }
}

impl<S: Splittable> Debug for AsyncStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncStream").finish_non_exhaustive()
    }
}

struct WakerArray<const N: usize>([Option<Waker>; N]);

impl<const N: usize> Wake for WakerArray<N> {
    fn wake(self: Arc<Self>) {
        self.0.iter().for_each(|w| {
            if let Some(w) = w {
                w.wake_by_ref()
            }
        });
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
