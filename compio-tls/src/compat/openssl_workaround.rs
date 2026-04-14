//! Workarounds for OpenSSL, which doesn't expect the underlying stream to
//! return `Poll::Pending` from `poll_flush`.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{AsyncRead, AsyncWrite};
use pin_project_lite::pin_project;

pin_project! {
    #[derive(Debug)]
    pub struct OpensslInner<S> {
        #[pin]
        inner: S,
        written: Option<usize>,
    }
}

impl<S> OpensslInner<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            written: None,
        }
    }

    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

impl<S: AsyncRead> AsyncRead for OpensslInner<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.project().inner.poll_read(cx, buf)
    }
}

impl<S: AsyncWrite> AsyncWrite for OpensslInner<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let this = self.as_mut().project();
            match *this.written {
                None => match this.inner.poll_write(cx, buf) {
                    Poll::Ready(Ok(n)) => {
                        *this.written = Some(n);
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                },
                Some(n) => match this.inner.poll_flush(cx) {
                    Poll::Ready(Ok(())) => {
                        *this.written = None;
                        return Poll::Ready(Ok(n));
                    }
                    Poll::Ready(Err(e)) => {
                        *this.written = None;
                        return Poll::Ready(Err(e));
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_close(cx)
    }
}
