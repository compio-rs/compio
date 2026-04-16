//! Workarounds for native-tls like streams.
//!
//! * If it is handshaking, `poll_read` will flush the write buffer before
//!   reading, and `poll_flush` will do nothing.
//! * After handshaking, it behaves like a normal stream.

use std::{
    io::{self, Read, Write},
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{AsyncRead, AsyncWrite};
use pin_project_lite::pin_project;

pin_project! {
    #[derive(Debug)]
    struct OpensslInner<S> {
        #[pin]
        inner: S,
        written: bool,
        handshaken: bool,
    }
}

impl<S> OpensslInner<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            written: false,
            handshaken: false,
        }
    }

    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

impl<S: AsyncRead + AsyncWrite> AsyncRead for OpensslInner<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let this = self.as_mut().project();
            if !*this.handshaken && *this.written {
                match this.inner.poll_flush(cx) {
                    Poll::Pending => break Poll::Pending,
                    Poll::Ready(Ok(())) => {
                        *this.written = false;
                    }
                    Poll::Ready(Err(e)) => break Poll::Ready(Err(e)),
                }
            } else {
                break this.inner.poll_read(cx, buf);
            }
        }
    }
}

impl<S: AsyncWrite> AsyncWrite for OpensslInner<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.as_mut().project();
        let res = this.inner.poll_write(cx, buf);
        if let Poll::Ready(Ok(_)) = &res {
            *this.written = true;
        }
        res
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.handshaken {
            self.project().inner.poll_flush(cx)
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_close(cx)
    }
}

#[derive(Debug)]
pub struct AllowStd<S> {
    inner: OpensslInner<S>,
    context: *mut (),
}

impl<S> AllowStd<S> {
    pub fn new(inner: S, cx: &mut Context<'_>) -> Self {
        Self {
            inner: OpensslInner::new(inner),
            context: cx as *mut _ as *mut (),
        }
    }

    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
    }

    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut()
    }

    pub fn set_context(&mut self, cx: &mut Context<'_>) {
        self.context = cx as *mut _ as *mut ();
    }

    pub fn clear_context(&mut self) {
        self.context = std::ptr::null_mut();
    }

    pub fn finish_handshake(&mut self) {
        self.inner.handshaken = true;
    }
}

// *mut () context is neither Send nor Sync
unsafe impl<S: Send> Send for AllowStd<S> {}
unsafe impl<S: Sync> Sync for AllowStd<S> {}

impl<S> AllowStd<S>
where
    S: Unpin,
{
    fn with_context<F, R>(&mut self, f: F) -> io::Result<R>
    where
        F: FnOnce(&mut Context<'_>, Pin<&mut OpensslInner<S>>) -> Poll<io::Result<R>>,
    {
        unsafe {
            assert!(!self.context.is_null());
            let waker = &mut *(self.context as *mut _);
            match f(waker, Pin::new(&mut self.inner)) {
                Poll::Ready(r) => r,
                Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            }
        }
    }
}

impl<S> Read for AllowStd<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.with_context(|ctx, stream| stream.poll_read(ctx, buf))
    }
}

impl<S> Write for AllowStd<S>
where
    S: AsyncWrite + Unpin,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.with_context(|ctx, stream| stream.poll_write(ctx, buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.with_context(|ctx, stream| stream.poll_flush(ctx))
    }
}
