//! Ported from `tokio-native-tls`.

use std::{
    fmt,
    future::Future,
    io::{self, Read, Write},
    marker::Unpin,
    pin::Pin,
    ptr::null_mut,
    task::{Context, Poll},
};

use futures_util::{AsyncRead, AsyncWrite};
use native_tls::{Error, HandshakeError, MidHandshakeTlsStream};

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
pub struct TlsStream<S>(native_tls::TlsStream<AllowStd<S>>);

#[derive(Clone)]
pub struct TlsConnector(native_tls::TlsConnector);

#[derive(Clone)]
pub struct TlsAcceptor(native_tls::TlsAcceptor);

struct MidHandshake<S>(Option<MidHandshakeTlsStream<AllowStd<S>>>);

#[allow(clippy::large_enum_variant)]
enum StartedHandshake<S> {
    Done(TlsStream<S>),
    Mid(MidHandshakeTlsStream<AllowStd<S>>),
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
        F: FnOnce(&mut native_tls::TlsStream<AllowStd<S>>) -> io::Result<R>,
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

    pub fn get_ref(&self) -> &native_tls::TlsStream<AllowStd<S>> {
        &self.0
    }

    pub fn get_mut(&mut self) -> &mut native_tls::TlsStream<AllowStd<S>> {
        &mut self.0
    }
}

impl<S> TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn negotiated_alpn(&self) -> native_tls::Result<Option<Vec<u8>>> {
        self.0.negotiated_alpn()
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
        self.with_context(ctx, |s| s.read(buf))
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
        self.with_context(ctx, |s| s.write(buf))
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with_context(ctx, |s| s.flush())
    }

    fn poll_close(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.with_context(ctx, |s| s.shutdown())
    }
}

async fn handshake<F, S>(f: F, stream: S) -> Result<TlsStream<S>, Error>
where
    F: FnOnce(
            AllowStd<S>,
        ) -> Result<native_tls::TlsStream<AllowStd<S>>, HandshakeError<AllowStd<S>>>
        + Unpin,
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
    F: FnOnce(
            AllowStd<S>,
        ) -> Result<native_tls::TlsStream<AllowStd<S>>, HandshakeError<AllowStd<S>>>
        + Unpin,
    S: Unpin,
    AllowStd<S>: Read + Write,
{
    type Output = Result<StartedHandshake<S>, Error>;

    fn poll(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Result<StartedHandshake<S>, Error>> {
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
            Err(HandshakeError::WouldBlock(mut s)) => {
                s.get_mut().context = null_mut();
                Poll::Ready(Ok(StartedHandshake::Mid(s)))
            }
            Err(HandshakeError::Failure(e)) => Poll::Ready(Err(e)),
        }
    }
}

impl TlsConnector {
    pub async fn connect<S>(&self, domain: &str, stream: S) -> Result<TlsStream<S>, Error>
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

impl From<native_tls::TlsConnector> for TlsConnector {
    fn from(inner: native_tls::TlsConnector) -> TlsConnector {
        TlsConnector(inner)
    }
}

impl TlsAcceptor {
    pub async fn accept<S>(&self, stream: S) -> Result<TlsStream<S>, Error>
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

impl From<native_tls::TlsAcceptor> for TlsAcceptor {
    fn from(inner: native_tls::TlsAcceptor) -> TlsAcceptor {
        TlsAcceptor(inner)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Future for MidHandshake<S> {
    type Output = Result<TlsStream<S>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut_self = self.get_mut();
        let mut s = mut_self.0.take().expect("future polled after completion");

        s.get_mut().context = cx as *mut _ as *mut ();
        match s.handshake() {
            Ok(mut s) => {
                s.get_mut().context = null_mut();
                Poll::Ready(Ok(TlsStream(s)))
            }
            Err(HandshakeError::WouldBlock(mut s)) => {
                s.get_mut().context = null_mut();
                mut_self.0 = Some(s);
                Poll::Pending
            }
            Err(HandshakeError::Failure(e)) => Poll::Ready(Err(e)),
        }
    }
}
