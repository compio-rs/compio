use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_io::{AsyncRead, AsyncWrite};
use compio_net::TcpStream;
use compio_tls::{TlsConnector, TlsStream};
use hyper::{
    client::connect::{Connected, Connection},
    Uri,
};
use send_wrapper::SendWrapper;

enum HttpStreamInner {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl HttpStreamInner {
    pub async fn new(uri: Uri) -> io::Result<Self> {
        let scheme = uri.scheme_str().unwrap_or("http");
        let host = uri
            .host()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "should specify host"))?;
        let port = uri.port_u16();
        match scheme {
            "http" => {
                let stream = TcpStream::connect((host, port.unwrap_or(80))).await?;
                Ok(Self::Tcp(stream))
            }
            "https" => {
                let stream = TcpStream::connect((host, port.unwrap_or(443))).await?;
                let connector = TlsConnector::from(
                    native_tls::TlsConnector::new()
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?,
                );
                Ok(Self::Tls(connector.connect(host, stream).await?))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported scheme",
            )),
        }
    }
}

impl AsyncRead for HttpStreamInner {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        match self {
            Self::Tcp(s) => s.read(buf).await,
            Self::Tls(s) => s.read(buf).await,
        }
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        match self {
            Self::Tcp(s) => s.read_vectored(buf).await,
            Self::Tls(s) => s.read_vectored(buf).await,
        }
    }
}

impl AsyncWrite for HttpStreamInner {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        match self {
            Self::Tcp(s) => s.write(buf).await,
            Self::Tls(s) => s.write(buf).await,
        }
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        match self {
            Self::Tcp(s) => s.write_vectored(buf).await,
            Self::Tls(s) => s.write_vectored(buf).await,
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush().await,
            Self::Tls(s) => s.flush().await,
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.shutdown().await,
            Self::Tls(s) => s.shutdown().await,
        }
    }
}

type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pub struct HttpStream {
    inner: HttpStreamInner,
    read_future: Option<PinBoxFuture<BufResult<usize, Vec<u8>>>>,
    write_future: Option<PinBoxFuture<BufResult<usize, Vec<u8>>>>,
    flush_future: Option<PinBoxFuture<io::Result<()>>>,
    shutdown_future: Option<PinBoxFuture<io::Result<()>>>,
}

impl HttpStream {
    pub async fn new(uri: Uri) -> io::Result<Self> {
        Ok(Self {
            inner: HttpStreamInner::new(uri).await?,
            read_future: None,
            write_future: None,
            flush_future: None,
            shutdown_future: None,
        })
    }
}

macro_rules! poll_future {
    ($f:expr, $cx:expr, $e:expr) => {{
        let mut future = match $f.take() {
            Some(f) => f,
            None => Box::pin(SendWrapper::new($e)),
        };
        let f = future.as_mut();
        match f.poll($cx) {
            Poll::Pending => {
                $f = Some(future);
                return Poll::Pending;
            }
            Poll::Ready(res) => res,
        }
    }};
}

impl tokio::io::AsyncRead for HttpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let inner: &'static mut HttpStreamInner = unsafe { &mut *(&mut self.inner as *mut _) };
        let BufResult(res, inner_buf) = poll_future!(
            self.read_future,
            cx,
            inner.read(Vec::with_capacity(unsafe { buf.unfilled_mut() }.len()))
        );
        res?;
        buf.put_slice(&inner_buf);
        Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncWrite for HttpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let inner: &'static mut HttpStreamInner = unsafe { &mut *(&mut self.inner as *mut _) };
        let BufResult(res, _) = poll_future!(self.write_future, cx, inner.write(buf.to_vec()));
        Poll::Ready(res)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner: &'static mut HttpStreamInner = unsafe { &mut *(&mut self.inner as *mut _) };
        let res = poll_future!(self.flush_future, cx, inner.flush());
        Poll::Ready(res)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner: &'static mut HttpStreamInner = unsafe { &mut *(&mut self.inner as *mut _) };
        let res = poll_future!(self.shutdown_future, cx, inner.shutdown());
        Poll::Ready(res)
    }
}

impl Connection for HttpStream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

unsafe impl Send for HttpStream {}
