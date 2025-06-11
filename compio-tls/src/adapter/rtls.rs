use std::{io, ops::DerefMut, sync::Arc};

use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
use rustls::{
    ClientConfig, ClientConnection, ConnectionCommon, Error, ServerConfig, ServerConnection,
    pki_types::ServerName,
};

use crate::TlsStream;

pub enum HandshakeError<S, C> {
    Rustls(Error),
    System(io::Error),
    WouldBlock(MidStream<S, C>),
}

pub struct MidStream<S, C> {
    stream: SyncStream<S>,
    conn: C,
    result_fn: fn(SyncStream<S>, C) -> TlsStream<S>,
}

impl<S, C> MidStream<S, C> {
    pub fn new(
        stream: SyncStream<S>,
        conn: C,
        result_fn: fn(SyncStream<S>, C) -> TlsStream<S>,
    ) -> Self {
        Self {
            stream,
            conn,
            result_fn,
        }
    }

    pub fn get_mut(&mut self) -> &mut SyncStream<S> {
        &mut self.stream
    }

    pub fn handshake<D>(mut self) -> Result<TlsStream<S>, HandshakeError<S, C>>
    where
        C: DerefMut<Target = ConnectionCommon<D>>,
        S: AsyncRead + AsyncWrite,
    {
        loop {
            let mut write_would_block = false;
            let mut read_would_block = false;

            while self.conn.wants_write() {
                match self.conn.write_tls(&mut self.stream) {
                    Ok(_) => {
                        write_would_block = true;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        write_would_block = true;
                        break;
                    }
                    Err(e) => return Err(HandshakeError::System(e)),
                }
            }

            while !self.stream.is_eof() && self.conn.wants_read() {
                match self.conn.read_tls(&mut self.stream) {
                    Ok(_) => {
                        self.conn
                            .process_new_packets()
                            .map_err(HandshakeError::Rustls)?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        read_would_block = true;
                        break;
                    }
                    Err(e) => return Err(HandshakeError::System(e)),
                }
            }

            return match (self.stream.is_eof(), self.conn.is_handshaking()) {
                (true, true) => {
                    let err = io::Error::new(io::ErrorKind::UnexpectedEof, "tls handshake eof");
                    Err(HandshakeError::System(err))
                }
                (_, false) => Ok((self.result_fn)(self.stream, self.conn)),
                (_, true) if write_would_block || read_would_block => {
                    Err(HandshakeError::WouldBlock(self))
                }
                _ => continue,
            };
        }
    }
}

#[derive(Debug, Clone)]
pub struct TlsConnector(pub Arc<ClientConfig>);

impl TlsConnector {
    #[allow(clippy::result_large_err)]
    pub fn connect<S: AsyncRead + AsyncWrite>(
        &self,
        domain: &str,
        stream: S,
    ) -> Result<TlsStream<S>, HandshakeError<S, ClientConnection>> {
        let conn = ClientConnection::new(
            self.0.clone(),
            ServerName::try_from(domain)
                .map_err(|e| HandshakeError::System(io::Error::other(e)))?
                .to_owned(),
        )
        .map_err(HandshakeError::Rustls)?;

        MidStream::new(
            SyncStream::new(stream),
            conn,
            TlsStream::<S>::new_rustls_client,
        )
        .handshake()
    }
}

#[derive(Debug, Clone)]
pub struct TlsAcceptor(pub Arc<ServerConfig>);

impl TlsAcceptor {
    #[allow(clippy::result_large_err)]
    pub fn accept<S: AsyncRead + AsyncWrite>(
        &self,
        stream: S,
    ) -> Result<TlsStream<S>, HandshakeError<S, ServerConnection>> {
        let conn = ServerConnection::new(self.0.clone()).map_err(HandshakeError::Rustls)?;

        MidStream::new(
            SyncStream::new(stream),
            conn,
            TlsStream::<S>::new_rustls_server,
        )
        .handshake()
    }
}
