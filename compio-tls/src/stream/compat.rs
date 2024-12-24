//! The code here should sync with `compio::io::compat`.

use std::{
    future::Future,
    io,
    mem::MaybeUninit,
    pin::Pin,
    task::{Context, Poll},
};

use compio_io::{AsyncRead, AsyncWrite};
use pin_project_lite::pin_project;

use crate::TlsStream;

type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pin_project! {
    /// A [`TlsStream`] wrapper for [`futures_util::io`] traits.
    pub struct TlsStreamCompat<S> {
        #[pin]
        inner: TlsStream<S>,
        read_future: Option<PinBoxFuture<io::Result<usize>>>,
        write_future: Option<PinBoxFuture<io::Result<usize>>>,
        shutdown_future: Option<PinBoxFuture<io::Result<()>>>,
    }
}

impl<S> TlsStreamCompat<S> {
    /// Create [`TlsStreamCompat`] from [`TlsStream`].
    pub fn new(stream: TlsStream<S>) -> Self {
        Self {
            inner: stream,
            read_future: None,
            write_future: None,
            shutdown_future: None,
        }
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &TlsStream<S> {
        &self.inner
    }
}

impl<S> From<TlsStream<S>> for TlsStreamCompat<S> {
    fn from(value: TlsStream<S>) -> Self {
        Self::new(value)
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

impl<S: AsyncRead + 'static> futures_util::AsyncRead for TlsStreamCompat<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        let inner: &'static mut TlsStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };

        poll_future_would_block!(
            this.read_future,
            cx,
            inner.0.get_mut().fill_read_buf(),
            io::Read::read(&mut inner.0, buf)
        )
    }
}

impl<S: AsyncRead + 'static> TlsStreamCompat<S> {
    /// Attempt to read from the `AsyncRead` into `buf`.
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))`.
    pub fn poll_read_uninit(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();

        let inner: &'static mut TlsStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        poll_future_would_block!(
            this.read_future,
            cx,
            inner.0.get_mut().fill_read_buf(),
            super::read_buf(&mut inner.0, buf)
        )
    }
}

impl<S: AsyncWrite + 'static> futures_util::AsyncWrite for TlsStreamCompat<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();

        if this.shutdown_future.is_some() {
            debug_assert!(this.write_future.is_none());
            return Poll::Pending;
        }

        let inner: &'static mut TlsStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        poll_future_would_block!(
            this.write_future,
            cx,
            inner.0.get_mut().flush_write_buf(),
            io::Write::write(&mut inner.0, buf)
        )
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();

        if this.shutdown_future.is_some() {
            debug_assert!(this.write_future.is_none());
            return Poll::Pending;
        }

        let inner: &'static mut TlsStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        let res = poll_future!(this.write_future, cx, inner.0.get_mut().flush_write_buf());
        Poll::Ready(res.map(|_| ()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();

        // Avoid shutdown on flush because the inner buffer might be passed to the
        // driver.
        if this.write_future.is_some() {
            debug_assert!(this.shutdown_future.is_none());
            return Poll::Pending;
        }

        let inner: &'static mut TlsStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        let res = poll_future!(
            this.shutdown_future,
            cx,
            inner.0.get_mut().get_mut().shutdown()
        );
        Poll::Ready(res)
    }
}
