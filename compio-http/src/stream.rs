use std::io;

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::TcpStream;
use compio_tls::{TlsConnector, TlsStream};
use http::{
    header::CONTENT_LENGTH, request, uri::Scheme, HeaderName, HeaderValue, Response, StatusCode,
    Version,
};

pub enum HttpStream {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl HttpStream {
    pub async fn new(scheme: &Scheme, host: &str, port: Option<u16>) -> io::Result<Self> {
        match scheme.as_str() {
            "http" => Ok(Self::Tcp(
                TcpStream::connect((host, port.unwrap_or(80))).await?,
            )),
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

    pub async fn write_request_parts(&mut self, parts: &request::Parts) -> io::Result<()> {
        self.write_all(parts.method.to_string()).await.0?;
        self.write_all(" ").await.0?;
        self.write_all(
            parts
                .uri
                .path_and_query()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        )
        .await
        .0?;
        self.write_all(" ").await.0?;
        self.write_all(format!("{:?}", parts.version)).await.0?;
        self.write_all("\r\n").await.0?;
        for (header_name, header_value) in &parts.headers {
            self.write_all(header_name.to_string()).await.0?;
            self.write_all(": ").await.0?;
            self.write_all(header_value.as_bytes().to_vec()).await.0?;
            self.write_all("\r\n").await.0?;
        }
        self.write_all("\r\n").await.0?;
        Ok(())
    }

    pub async fn read_response(&mut self) -> io::Result<Response<Vec<u8>>> {
        let mut buffer = Vec::with_capacity(1024);
        'read_loop: loop {
            let len = buffer.len();
            let BufResult(res, slice) = self.read(buffer.slice(len..)).await;
            res?;
            buffer = slice.into_inner();

            let mut header_buffer = vec![httparse::EMPTY_HEADER; 16];
            'parse_loop: loop {
                let mut response = httparse::Response::new(&mut header_buffer);
                match response.parse(&buffer) {
                    Ok(status) => match status {
                        httparse::Status::Complete(len) => {
                            let mut resp = Response::new(vec![]);
                            *resp.version_mut() = match response.version.unwrap_or(1) {
                                0 => Version::HTTP_10,
                                1 => Version::HTTP_11,
                                _ => Version::HTTP_09,
                            };
                            *resp.status_mut() = StatusCode::from_u16(response.code.unwrap_or(200))
                                .expect("server should return a valid status code");
                            let headers = resp.headers_mut();
                            for header in response.headers {
                                if !header.name.is_empty() {
                                    let name = HeaderName::from_bytes(header.name.as_bytes())
                                        .expect("server should return a valid header name");
                                    let value = HeaderValue::from_bytes(header.value)
                                        .expect("server should return a valid header value");
                                    headers.append(name, value);
                                }
                            }
                            let full_len = headers
                                .get(CONTENT_LENGTH)
                                .map(|v| {
                                    std::str::from_utf8(v.as_bytes())
                                        .expect("content length should be valid utf8")
                                        .parse::<usize>()
                                        .expect("content length should be a integer")
                                })
                                .expect("should contain content length");
                            let mut buffer = buffer[len..].to_vec();
                            let curr_len = buffer.len();
                            if curr_len < full_len {
                                buffer.reserve(full_len - curr_len);
                                let BufResult(res, slice) =
                                    self.read_exact(buffer.slice(curr_len..full_len)).await;
                                res?;
                                buffer = slice.into_inner();
                            }
                            *resp.body_mut() = buffer;
                            return Ok(resp);
                        }
                        httparse::Status::Partial => continue 'read_loop,
                    },
                    Err(e) => match e {
                        httparse::Error::TooManyHeaders => {
                            header_buffer.resize(16, httparse::EMPTY_HEADER);
                            continue 'parse_loop;
                        }
                        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                    },
                }
            }
        }
    }
}

impl AsyncRead for HttpStream {
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

impl AsyncWrite for HttpStream {
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
