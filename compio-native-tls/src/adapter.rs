use std::io;

use compio_io::{AsyncRead, AsyncWrite};
use native_tls::HandshakeError;

use crate::{wrapper::StreamWrapper, TlsStream};

/// A wrapper around a [`native_tls::TlsConnector`], providing an async
/// `connect` method.
///
/// ```rust
/// use compio_io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
/// use compio_native_tls::TlsConnector;
/// use compio_net::TcpStream;
///
/// # compio_runtime::block_on(async {
/// let connector = TlsConnector::from(native_tls::TlsConnector::new().unwrap());
///
/// let stream = TcpStream::connect("www.bing.com:443").await.unwrap();
/// let mut stream = connector.connect("www.bing.com", stream).await.unwrap();
///
/// stream
///     .write_all("GET / HTTP/1.1\r\nHost:www.bing.com\r\nConnection: close\r\n\r\n")
///     .await
///     .unwrap();
/// stream.flush().await.unwrap();
/// let (_, res) = stream.read_to_end(vec![]).await.unwrap();
/// println!("{}", String::from_utf8_lossy(&res));
/// # })
/// ```
#[derive(Debug, Clone)]
pub struct TlsConnector(native_tls::TlsConnector);

impl From<native_tls::TlsConnector> for TlsConnector {
    fn from(value: native_tls::TlsConnector) -> Self {
        Self(value)
    }
}

impl TlsConnector {
    /// Connects the provided stream with this connector, assuming the provided
    /// domain.
    ///
    /// This function will internally call `TlsConnector::connect` to connect
    /// the stream and returns a future representing the resolution of the
    /// connection operation. The returned future will resolve to either
    /// `TlsStream<S>` or `Error` depending if it's successful or not.
    ///
    /// This is typically used for clients who have already established, for
    /// example, a TCP connection to a remote server. That stream is then
    /// provided here to perform the client half of a connection to a
    /// TLS-powered server.
    pub async fn connect<S: AsyncRead + AsyncWrite>(
        &self,
        domain: &str,
        stream: S,
    ) -> io::Result<TlsStream<S>> {
        handshake(self.0.connect(domain, StreamWrapper::new(stream))).await
    }
}

/// A wrapper around a [`native_tls::TlsAcceptor`], providing an async `accept`
/// method.
#[derive(Clone)]
pub struct TlsAcceptor(native_tls::TlsAcceptor);

impl From<native_tls::TlsAcceptor> for TlsAcceptor {
    fn from(value: native_tls::TlsAcceptor) -> Self {
        Self(value)
    }
}

impl TlsAcceptor {
    /// Accepts a new client connection with the provided stream.
    ///
    /// This function will internally call `TlsAcceptor::accept` to connect
    /// the stream and returns a future representing the resolution of the
    /// connection operation. The returned future will resolve to either
    /// `TlsStream<S>` or `Error` depending if it's successful or not.
    ///
    /// This is typically used after a new socket has been accepted from a
    /// `TcpListener`. That socket is then passed to this function to perform
    /// the server half of accepting a client connection.
    pub async fn accept<S: AsyncRead + AsyncWrite>(&self, stream: S) -> io::Result<TlsStream<S>> {
        handshake(self.0.accept(StreamWrapper::new(stream))).await
    }
}

async fn handshake<S: AsyncRead + AsyncWrite>(
    mut res: Result<native_tls::TlsStream<StreamWrapper<S>>, HandshakeError<StreamWrapper<S>>>,
) -> io::Result<TlsStream<S>> {
    loop {
        match res {
            Ok(s) => return Ok(TlsStream::from(s)),
            Err(e) => match e {
                HandshakeError::Failure(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
                HandshakeError::WouldBlock(mut mid_stream) => {
                    if mid_stream.get_mut().flush_write_buf().await? == 0 {
                        mid_stream.get_mut().fill_read_buf().await?;
                    }
                    res = mid_stream.handshake();
                }
            },
        }
    }
}
