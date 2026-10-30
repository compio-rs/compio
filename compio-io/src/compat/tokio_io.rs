//! [`tokio::io`] traits for the [`futures_util::io`] adapters, so that crates
//! written against them (`h2`, `tokio-rustls`, `tonic`...) run on compio
//! streams without a tokio runtime.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use tokio::io::ReadBuf;

use crate::{
    AsyncRead, AsyncWrite,
    compat::{AsyncReadStream, AsyncStream, AsyncWriteStream},
    util::Splittable,
};

/// Reads into the unfilled part of `buf` without initializing it first.
///
/// # Safety
///
/// `read` must not de-initialize any byte of the slice it is given, and when
/// it returns `Ok(n)`, it must have initialized the first `n` bytes of it.
unsafe fn read_uninit(
    buf: &mut ReadBuf<'_>,
    read: impl FnOnce(&mut [std::mem::MaybeUninit<u8>]) -> Poll<io::Result<usize>>,
) -> Poll<io::Result<()>> {
    // SAFETY: the caller guarantees `read` does not de-initialize the slice.
    let n = ready!(read(unsafe { buf.unfilled_mut() }))?;
    // SAFETY: the caller guarantees `read` initialized the first `n` bytes.
    unsafe { buf.assume_init(n) };
    buf.advance(n);
    Poll::Ready(Ok(()))
}

impl<S: AsyncRead + Unpin + 'static> tokio::io::AsyncRead for AsyncReadStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // SAFETY: `poll_read_uninit` only copies initialized bytes into the
        // slice, and returns how many it copied to its start.
        unsafe { read_uninit(buf, |unfilled| self.poll_read_uninit(cx, unfilled)) }
    }
}

impl<S: AsyncRead + Unpin + 'static> tokio::io::AsyncBufRead for AsyncReadStream<S> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        futures_util::AsyncBufRead::poll_fill_buf(self, cx)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        futures_util::AsyncBufRead::consume(self, amt)
    }
}

impl<S: AsyncWrite + Unpin + 'static> tokio::io::AsyncWrite for AsyncWriteStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        futures_util::AsyncWrite::poll_write(self, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::AsyncWrite::poll_flush(self, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::AsyncWrite::poll_close(self, cx)
    }
}

impl<S: Splittable + 'static> tokio::io::AsyncRead for AsyncStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // SAFETY: `poll_read_uninit` only copies initialized bytes into the
        // slice, and returns how many it copied to its start.
        unsafe { read_uninit(buf, |unfilled| self.poll_read_uninit(cx, unfilled)) }
    }
}

impl<S: Splittable + 'static> tokio::io::AsyncBufRead for AsyncStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        futures_util::AsyncBufRead::poll_fill_buf(self, cx)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        futures_util::AsyncBufRead::consume(self, amt)
    }
}

impl<S: Splittable + 'static> tokio::io::AsyncWrite for AsyncStream<S>
where
    S::WriteHalf: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        futures_util::AsyncWrite::poll_write(self, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::AsyncWrite::poll_flush(self, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::AsyncWrite::poll_close(self, cx)
    }
}
