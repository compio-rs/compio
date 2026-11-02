//! [`tokio::io`] traits for the TLS streams, so that crates written against
//! them (`h2`, `tonic`...) run over compio TLS without a tokio runtime.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_util::{AsyncRead, AsyncWrite};
use tokio::io::ReadBuf;

use crate::{MaybeTlsStream, TlsStream};

macro_rules! impl_tokio {
    ($t:ident) => {
        impl<S> tokio::io::AsyncRead for $t<S>
        where
            S: AsyncRead + AsyncWrite + Unpin,
        {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                let n = ready!(AsyncRead::poll_read(self, cx, buf.initialize_unfilled()))?;
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
        }

        impl<S> tokio::io::AsyncWrite for $t<S>
        where
            S: AsyncRead + AsyncWrite + Unpin,
        {
            fn poll_write(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<io::Result<usize>> {
                AsyncWrite::poll_write(self, cx, buf)
            }

            fn poll_write_vectored(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                bufs: &[io::IoSlice<'_>],
            ) -> Poll<io::Result<usize>> {
                AsyncWrite::poll_write_vectored(self, cx, bufs)
            }

            fn is_write_vectored(&self) -> bool {
                self.writes_vectored()
            }

            fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                AsyncWrite::poll_flush(self, cx)
            }

            fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                AsyncWrite::poll_close(self, cx)
            }
        }
    };
}

impl_tokio!(TlsStream);
impl_tokio!(MaybeTlsStream);
