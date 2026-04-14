//! Workarounds for OpenSSL, which doesn't expect the underlying stream to
//! return `Poll::Pending` from `poll_flush`.

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
    S: AsyncRead + Unpin,
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
