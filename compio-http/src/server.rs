use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{ready, Context, Poll},
};

use compio_net::{TcpListener, TcpStream};
use compio_tls::TlsStream;
use hyper::server::accept::Accept;

use crate::HttpStream;

type LocalPinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// A TCP acceptor.
pub struct Acceptor {
    listener: TcpListener,
    fut: Option<LocalPinBoxFuture<io::Result<(TcpStream, SocketAddr)>>>,
}

impl Acceptor {
    /// Create [`Acceptor`] binding to provided socket address.
    pub async fn bind(addr: &SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self::from_listener(listener))
    }

    /// Create [`Acceptor`] from an existing [`compio_net::TcpListener`].
    pub fn from_listener(listener: TcpListener) -> Self {
        Self {
            listener,
            fut: None,
        }
    }

    fn poll_accept_impl(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<TcpStream>> {
        let listener: &'static TcpListener = unsafe { &*(&self.listener as *const _) };
        if let Some(mut fut) = self.fut.take() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    self.fut = Some(fut);
                    Poll::Pending
                }
                Poll::Ready(res) => Poll::Ready(res.map(|(s, _)| s)),
            }
        } else {
            self.fut = Some(Box::pin(listener.accept()));
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

impl Accept for Acceptor {
    type Conn = HttpStream;
    type Error = io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let res = ready!(self.poll_accept_impl(cx));
        Poll::Ready(Some(res.map(HttpStream::from_tcp)))
    }
}

/// A TLS acceptor.
pub struct TlsAcceptor {
    tcp_acceptor: Acceptor,
    tls_acceptor: compio_tls::TlsAcceptor,
    fut: Option<LocalPinBoxFuture<io::Result<TlsStream<TcpStream>>>>,
}

impl TlsAcceptor {
    /// Create [`TlsAcceptor`] from an existing [`compio_net::TcpListener`] and
    /// [`compio_tls::TlsAcceptor`].
    pub fn from_listener(tcp_listener: TcpListener, tls_acceptor: compio_tls::TlsAcceptor) -> Self {
        Self {
            tcp_acceptor: Acceptor::from_listener(tcp_listener),
            tls_acceptor,
            fut: None,
        }
    }
}

impl Accept for TlsAcceptor {
    type Conn = HttpStream;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let acceptor: &'static compio_tls::TlsAcceptor =
            unsafe { &*(&self.tls_acceptor as *const _) };
        if let Some(mut fut) = self.fut.take() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    self.fut = Some(fut);
                    Poll::Pending
                }
                Poll::Ready(res) => Poll::Ready(Some(res.map(HttpStream::from_tls))),
            }
        } else {
            let tcp_acceptor = Pin::new(&mut self.tcp_acceptor);
            let res = ready!(tcp_acceptor.poll_accept_impl(cx));
            match res {
                Ok(stream) => {
                    self.fut = Some(Box::pin(acceptor.accept(stream)));
                }
                Err(e) => return Poll::Ready(Some(Err(e))),
            }
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
