use std::io;

use rustls::{ClientConnection, Error, IoState, Reader, ServerConnection, Writer};

#[derive(Debug)]
enum TlsConnection {
    Client(ClientConnection),
    Server(ServerConnection),
}

impl TlsConnection {
    pub fn reader(&mut self) -> Reader<'_> {
        match self {
            Self::Client(c) => c.reader(),
            Self::Server(c) => c.reader(),
        }
    }

    pub fn writer(&mut self) -> Writer<'_> {
        match self {
            Self::Client(c) => c.writer(),
            Self::Server(c) => c.writer(),
        }
    }

    pub fn process_new_packets(&mut self) -> Result<IoState, Error> {
        match self {
            Self::Client(c) => c.process_new_packets(),
            Self::Server(c) => c.process_new_packets(),
        }
    }

    pub fn read_tls(&mut self, rd: &mut dyn io::Read) -> io::Result<usize> {
        match self {
            Self::Client(c) => c.read_tls(rd),
            Self::Server(c) => c.read_tls(rd),
        }
    }

    pub fn wants_read(&self) -> bool {
        match self {
            Self::Client(c) => c.wants_read(),
            Self::Server(c) => c.wants_read(),
        }
    }

    pub fn write_tls(&mut self, wr: &mut dyn io::Write) -> io::Result<usize> {
        match self {
            Self::Client(c) => c.write_tls(wr),
            Self::Server(c) => c.write_tls(wr),
        }
    }

    pub fn wants_write(&self) -> bool {
        match self {
            Self::Client(c) => c.wants_write(),
            Self::Server(c) => c.wants_write(),
        }
    }
}

#[derive(Debug)]
pub struct TlsStream<S> {
    inner: S,
    conn: TlsConnection,
}

impl<S> TlsStream<S> {
    pub fn new_client(inner: S, conn: ClientConnection) -> Self {
        Self {
            inner,
            conn: TlsConnection::Client(conn),
        }
    }

    pub fn new_server(inner: S, conn: ServerConnection) -> Self {
        Self {
            inner,
            conn: TlsConnection::Server(conn),
        }
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn negotiated_alpn(&self) -> Option<&[u8]> {
        match &self.conn {
            TlsConnection::Client(client) => client.alpn_protocol(),
            TlsConnection::Server(server) => server.alpn_protocol(),
        }
    }

    pub fn is_handshaking(&self) -> bool {
        match &self.conn {
            TlsConnection::Client(client) => client.is_handshaking(),
            TlsConnection::Server(server) => server.is_handshaking(),
        }
    }
}

impl<S: io::Read> TlsStream<S> {
    fn read_impl<T>(&mut self, mut f: impl FnMut(Reader) -> io::Result<T>) -> io::Result<T> {
        let mut eof = false;
        while self.conn.wants_read() {
            let res = self.conn.read_tls(&mut self.inner)?;
            self.conn.process_new_packets().map_err(io::Error::other)?;
            if res == 0 {
                eof = true;
                break;
            }
        }

        if eof && self.is_handshaking() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "TLS handshake interrupted by EOF",
            ));
        }

        f(self.conn.reader())
    }
}

impl<S: io::Read> io::Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_impl(|mut reader| reader.read(buf))
    }

    #[cfg(feature = "read_buf")]
    fn read_buf(&mut self, mut buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        self.read_impl(|mut reader| reader.read_buf(buf.reborrow()))
    }
}

impl<S: io::Write> io::Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.flush()?;
        self.conn.writer().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        while self.conn.wants_write() {
            self.conn.write_tls(&mut self.inner)?;
        }
        self.inner.flush()?;
        Ok(())
    }
}
