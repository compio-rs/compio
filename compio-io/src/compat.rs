//! Compat wrappers for interop with other crates.

use std::{
    future::Future,
    io::{self, BufRead, Read, Write},
    mem::MaybeUninit,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit};
use pin_project_lite::pin_project;

use crate::{buffer::Buffer, util::DEFAULT_BUF_SIZE};

/// A wrapper for [`AsyncRead`](crate::AsyncRead) +
/// [`AsyncWrite`](crate::AsyncWrite), providing sync traits impl.
///
/// The sync methods will return [`io::ErrorKind::WouldBlock`] error if the
/// inner buffer needs more data.
#[derive(Debug)]
pub struct SyncStream<S> {
    stream: S,
    eof: bool,
    read_buffer: Buffer,
    write_buffer: Buffer,
}

impl<S> SyncStream<S> {
    /// Create [`SyncStream`] with the stream and default buffer size.
    pub fn new(stream: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, stream)
    }

    /// Create [`SyncStream`] with the stream and buffer size.
    pub fn with_capacity(cap: usize, stream: S) -> Self {
        Self {
            stream,
            eof: false,
            read_buffer: Buffer::with_capacity(cap),
            write_buffer: Buffer::with_capacity(cap),
        }
    }

    /// Get if the stream is at EOF.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    /// Get the mutable reference of the inner stream.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    fn flush_impl(&mut self) -> io::Result<()> {
        if !self.write_buffer.is_empty() {
            Err(would_block("need to flush the write buffer"))
        } else {
            Ok(())
        }
    }

    /// Pull some bytes from this source into the specified buffer.
    pub fn read_buf_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        let slice = self.fill_buf()?;
        let amt = buf.len().min(slice.len());
        // SAFETY: the length is valid
        buf[..amt]
            .copy_from_slice(unsafe { std::slice::from_raw_parts(slice.as_ptr().cast(), amt) });
        self.consume(amt);
        Ok(amt)
    }
}

impl<S> Read for SyncStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut slice = self.fill_buf()?;
        slice.read(buf).inspect(|res| {
            self.consume(*res);
        })
    }

    #[cfg(feature = "read_buf")]
    fn read_buf(&mut self, mut buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        let mut slice = self.fill_buf()?;
        let old_written = buf.written();
        slice.read_buf(buf.reborrow())?;
        let len = buf.written() - old_written;
        self.consume(len);
        Ok(())
    }
}

impl<S> BufRead for SyncStream<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.read_buffer.all_done() {
            self.read_buffer.reset();
        }

        if self.read_buffer.slice().is_empty() && !self.eof {
            return Err(would_block("need to fill the read buffer"));
        }

        Ok(self.read_buffer.slice())
    }

    fn consume(&mut self, amt: usize) {
        self.read_buffer.advance(amt);
    }
}

impl<S> Write for SyncStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.write_buffer.need_flush() {
            self.flush_impl()?;
        }

        let written = self.write_buffer.with_sync(|mut inner| {
            let len = buf.len().min(inner.buf_capacity() - inner.buf_len());
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buf.as_ptr(),
                    inner.as_buf_mut_ptr().add(inner.buf_len()),
                    len,
                );
                inner.set_buf_init(inner.buf_len() + len);
            }
            BufResult(Ok(len), inner)
        })?;

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Related PR:
        // https://github.com/sfackler/rust-openssl/pull/1922
        // After this PR merged, we can use self.flush_impl()
        Ok(())
    }
}

fn would_block(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, msg)
}

impl<S: crate::AsyncRead> SyncStream<S> {
    /// Fill the read buffer.
    pub async fn fill_read_buf(&mut self) -> io::Result<usize> {
        let stream = &mut self.stream;
        let len = self
            .read_buffer
            .with(|b| async move {
                let len = b.buf_len();
                let b = b.slice(len..);
                stream.read(b).await.into_inner()
            })
            .await?;
        if len == 0 {
            self.eof = true;
        }
        Ok(len)
    }
}

impl<S: crate::AsyncWrite> SyncStream<S> {
    /// Flush all data in the write buffer.
    pub async fn flush_write_buf(&mut self) -> io::Result<usize> {
        let stream = &mut self.stream;
        let len = self.write_buffer.flush_to(stream).await?;
        stream.flush().await?;
        Ok(len)
    }
}

type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pin_project! {
    /// A stream wrapper for [`futures_util::io`] traits.
    pub struct AsyncStream<S> {
        #[pin]
        inner: SyncStream<S>,
        read_future: Option<PinBoxFuture<io::Result<usize>>>,
        write_future: Option<PinBoxFuture<io::Result<usize>>>,
        shutdown_future: Option<PinBoxFuture<io::Result<()>>>,
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
        }
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
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

impl<S: crate::AsyncRead + 'static> futures_util::AsyncRead for AsyncStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();
        // Safety:
        // - The futures won't live longer than the stream.
        // - `self` is pinned.
        // - The inner stream won't be moved.
        let inner: &'static mut SyncStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };

        poll_future_would_block!(
            this.read_future,
            cx,
            inner.fill_read_buf(),
            io::Read::read(inner, buf)
        )
    }
}

impl<S: crate::AsyncRead + 'static> AsyncStream<S> {
    /// Attempt to read from the `AsyncRead` into `buf`.
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))`.
    pub fn poll_read_uninit(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<io::Result<usize>> {
        #[cfg(feature = "read_buf")]
        {
            let this = self.project();

            let inner: &'static mut SyncStream<S> =
                unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
            poll_future_would_block!(
                this.read_future,
                cx,
                inner.fill_read_buf(),
                inner.read_buf_uninit(buf)
            )
        }
        #[cfg(not(feature = "read_buf"))]
        {
            buf.fill(MaybeUninit::new(0));
            self.poll_read(cx, unsafe {
                std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len())
            })
        }
    }
}

impl<S: crate::AsyncRead + 'static> futures_util::AsyncBufRead for AsyncStream<S> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.project();

        let inner: &'static mut SyncStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        poll_future_would_block!(
            this.read_future,
            cx,
            inner.fill_read_buf(),
            // Safety: anyway the slice won't be used after free.
            io::BufRead::fill_buf(inner).map(|slice| unsafe { &*(slice as *const _) })
        )
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = self.project();

        let inner: &'static mut SyncStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        inner.consume(amt)
    }
}

impl<S: crate::AsyncWrite + 'static> futures_util::AsyncWrite for AsyncStream<S> {
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

        let inner: &'static mut SyncStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        poll_future_would_block!(
            this.write_future,
            cx,
            inner.flush_write_buf(),
            io::Write::write(inner, buf)
        )
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.project();

        if this.shutdown_future.is_some() {
            debug_assert!(this.write_future.is_none());
            return Poll::Pending;
        }

        let inner: &'static mut SyncStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        let res = poll_future!(this.write_future, cx, inner.flush_write_buf());
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

        let inner: &'static mut SyncStream<S> =
            unsafe { &mut *(this.inner.get_unchecked_mut() as *mut _) };
        let res = poll_future!(this.shutdown_future, cx, inner.get_mut().shutdown());
        Poll::Ready(res)
    }
}
