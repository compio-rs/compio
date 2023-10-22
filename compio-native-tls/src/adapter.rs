use std::io;

use compio_io::{AsyncRead, AsyncWrite};
use native_tls::HandshakeError;

use crate::{wrapper::StreamWrapper, TlsStream};

#[derive(Debug)]
pub struct TlsConnector(native_tls::TlsConnector);

impl From<native_tls::TlsConnector> for TlsConnector {
    fn from(value: native_tls::TlsConnector) -> Self {
        Self(value)
    }
}

impl TlsConnector {
    pub async fn connect<S: AsyncRead + AsyncWrite>(
        &self,
        domain: &str,
        stream: S,
    ) -> io::Result<TlsStream<S>> {
        handshake(self.0.connect(domain, StreamWrapper::new(stream))).await
    }
}

pub struct TlsAcceptor(native_tls::TlsAcceptor);

impl From<native_tls::TlsAcceptor> for TlsAcceptor {
    fn from(value: native_tls::TlsAcceptor) -> Self {
        Self(value)
    }
}

impl TlsAcceptor {
    pub async fn accept<S: AsyncRead + AsyncWrite>(&self, stream: S) -> io::Result<TlsStream<S>> {
        handshake(self.0.accept(StreamWrapper::new(stream))).await
    }
}

async fn handshake<S: AsyncRead + AsyncWrite>(
    res: Result<native_tls::TlsStream<StreamWrapper<S>>, HandshakeError<StreamWrapper<S>>>,
) -> io::Result<TlsStream<S>> {
    match res {
        Ok(s) => Ok(TlsStream::from(s)),
        Err(e) => match e {
            HandshakeError::Failure(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
            HandshakeError::WouldBlock(mut mid_stream) => loop {
                mid_stream.get_mut().fill_read_buf().await?;
                mid_stream.get_mut().flush_write_buf().await?;
                match mid_stream.handshake() {
                    Ok(s) => return Ok(TlsStream::from(s)),
                    Err(e) => match e {
                        HandshakeError::Failure(e) => {
                            return Err(io::Error::new(io::ErrorKind::Other, e));
                        }
                        HandshakeError::WouldBlock(s) => {
                            mid_stream = s;
                            continue;
                        }
                    },
                }
            },
        },
    }
}
