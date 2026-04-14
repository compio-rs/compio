use std::{
    fmt,
    future::Future,
    io::{self, Read, Write},
    marker::Unpin,
    pin::Pin,
    ptr::null_mut,
    task::{Context, Poll},
};

use compio_py_dynamic_openssl::{
    SSLContext,
    ssl::{ErrorCode, HandshakeError, MidHandshakeSslStream, SslStream},
};
use futures_util::{AsyncRead, AsyncWrite};

use super::openssl_workaround::OpensslInner;

#[derive(Debug)]
pub struct AllowStd<S> {
    inner: OpensslInner<S>,
    context: *mut (),
}

impl<S> AllowStd<S> {
    pub fn get_ref(&self) -> &S {
        self.inner.get_ref()
    }

    pub fn get_mut(&mut self) -> &mut S {
        self.inner.get_mut()
    }
}

#[derive(Debug)]
pub struct TlsStream<S>(SslStream<AllowStd<S>>);

#[derive(Clone)]
pub struct TlsConnector(SSLContext);

#[derive(Clone)]
pub struct TlsAcceptor(SSLContext);

struct MidHandshake<S>(Option<MidHandshakeSslStream<AllowStd<S>>>);

#[allow(clippy::large_enum_variant)]
enum StartedHandshake<S> {
    Done(TlsStream<S>),
    Mid(MidHandshakeSslStream<AllowStd<S>>),
}

struct StartedHandshakeFuture<F, S>(Option<StartedHandshakeFutureInner<F, S>>);
struct StartedHandshakeFutureInner<F, S> {
    f: F,
    stream: S,
}

struct Guard<'a, S>(&'a mut TlsStream<S>)
where
    AllowStd<S>: Read + Write;

impl<S> Drop for Guard<'_, S>
where
    AllowStd<S>: Read + Write,
{
    fn drop(&mut self) {
        (self.0).0.get_mut().context = null_mut();
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

impl<S> TlsStream<S> {
    fn with_context<F, R>(&mut self, ctx: &mut Context<'_>, f: F) -> Poll<io::Result<R>>
    where
        F: FnOnce(&mut SslStream<AllowStd<S>>) -> io::Result<R>,
        AllowStd<S>: Read + Write,
    {
        self.0.get_mut().context = ctx as *mut _ as *mut ();
        let g = Guard(self);
        match f(&mut (g.0).0) {
            Ok(v) => Poll::Ready(Ok(v)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    pub fn get_ref(&self) -> &SslStream<AllowStd<S>> {
        &self.0
    }

    pub fn get_mut(&mut self) -> &mut SslStream<AllowStd<S>> {
        &mut self.0
    }

    pub fn negotiated_alpn(&self) -> Option<&[u8]> {
        self.0.ssl().selected_alpn_protocol()
    }
}

impl<S> AsyncRead for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.with_context(ctx, |s| {
            loop {
                match s.ssl_read(buf) {
                    Ok(res) => return Ok(res),
                    Err(e) => match e.code() {
                        ErrorCode::ZERO_RETURN => return Ok(0),
                        ErrorCode::SYSCALL if e.io_error().is_none() => return Ok(0),
                        ErrorCode::WANT_READ if e.io_error().is_none() => {}
                        _ => return Err(e.into_io_error().unwrap_or_else(io::Error::other)),
                    },
                }
            }
        })
    }
}

impl<S> AsyncWrite for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.with_context(ctx, |s| {
            loop {
                match s.ssl_write(buf) {
                    Ok(res) => return Ok(res),
                    Err(e) if e.code() == ErrorCode::WANT_READ && e.io_error().is_none() => {}
                    Err(e) => return Err(e.into_io_error().unwrap_or_else(io::Error::other)),
                }
            }
        })
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with_context(ctx, |s| s.get_mut().flush())
    }

    fn poll_close(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with_context(ctx, |s| match s.shutdown() {
            Ok(_) => Ok(()),
            Err(e) if e.code() == ErrorCode::ZERO_RETURN => Ok(()),
            Err(e) => Err(e.into_io_error().unwrap_or_else(io::Error::other)),
        })
    }
}

async fn handshake<F, S>(f: F, stream: S) -> io::Result<TlsStream<S>>
where
    F: FnOnce(AllowStd<S>) -> Result<SslStream<AllowStd<S>>, HandshakeError<AllowStd<S>>> + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let start = StartedHandshakeFuture(Some(StartedHandshakeFutureInner { f, stream }));

    match start.await {
        Err(e) => Err(e),
        Ok(StartedHandshake::Done(s)) => Ok(s),
        Ok(StartedHandshake::Mid(s)) => MidHandshake(Some(s)).await,
    }
}

impl<F, S> Future for StartedHandshakeFuture<F, S>
where
    F: FnOnce(AllowStd<S>) -> Result<SslStream<AllowStd<S>>, HandshakeError<AllowStd<S>>> + Unpin,
    S: Unpin,
    AllowStd<S>: Read + Write,
{
    type Output = io::Result<StartedHandshake<S>>;

    fn poll(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<io::Result<StartedHandshake<S>>> {
        let inner = self.0.take().expect("future polled after completion");
        let stream = AllowStd {
            inner: OpensslInner::new(inner.stream),
            context: ctx as *mut _ as *mut (),
        };

        match (inner.f)(stream) {
            Ok(mut s) => {
                s.get_mut().context = null_mut();
                Poll::Ready(Ok(StartedHandshake::Done(TlsStream(s))))
            }
            Err(HandshakeError::SetupFailure(e)) => Poll::Ready(Err(io::Error::other(e))),
            Err(HandshakeError::WouldBlock(mut s)) => {
                s.get_mut().context = null_mut();
                Poll::Ready(Ok(StartedHandshake::Mid(s)))
            }
            Err(HandshakeError::Failure(e)) => Poll::Ready(Err(io::Error::other(e.into_error()))),
        }
    }
}

impl TlsConnector {
    pub async fn connect<S>(&self, domain: &str, stream: S) -> io::Result<TlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        handshake(move |s| self.0.connect(domain, s), stream).await
    }
}

impl fmt::Debug for TlsConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsConnector").finish()
    }
}

impl From<SSLContext> for TlsConnector {
    fn from(inner: SSLContext) -> TlsConnector {
        TlsConnector(inner)
    }
}

impl TlsAcceptor {
    pub async fn accept<S>(&self, stream: S) -> io::Result<TlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        handshake(move |s| self.0.accept(s), stream).await
    }
}

impl fmt::Debug for TlsAcceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsAcceptor").finish()
    }
}

impl From<SSLContext> for TlsAcceptor {
    fn from(inner: SSLContext) -> TlsAcceptor {
        TlsAcceptor(inner)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Future for MidHandshake<S> {
    type Output = io::Result<TlsStream<S>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut_self = self.get_mut();
        let mut s = mut_self.0.take().expect("future polled after completion");

        s.get_mut().context = cx as *mut _ as *mut ();
        match s.handshake() {
            Ok(mut s) => {
                s.get_mut().context = null_mut();
                Poll::Ready(Ok(TlsStream(s)))
            }
            Err(HandshakeError::SetupFailure(e)) => Poll::Ready(Err(io::Error::other(e))),
            Err(HandshakeError::WouldBlock(mut s)) => {
                s.get_mut().context = null_mut();
                mut_self.0 = Some(s);
                Poll::Pending
            }
            Err(HandshakeError::Failure(e)) => Poll::Ready(Err(io::Error::other(e.into_error()))),
        }
    }
}
