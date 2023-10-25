use std::io;

use compio_io::{AsyncWrite, AsyncWriteExt};
use http::{uri::Scheme, Request, Response};

use crate::HttpStream;

#[derive(Debug, Clone)]
pub struct Client {}

impl Client {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {}
    }

    pub async fn execute(&self, request: Request<Vec<u8>>) -> io::Result<Response<Vec<u8>>> {
        let uri = request.uri();
        let scheme = uri.scheme().unwrap_or(&Scheme::HTTP);
        let host = uri
            .host()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "must specify host"))?;
        let port = uri.port_u16();
        let mut stream = HttpStream::new(scheme, host, port).await?;

        let (parts, body) = request.into_parts();
        stream.write_request_parts(&parts).await?;
        stream.write_all(body).await.0?;
        stream.flush().await?;

        let response = stream.read_response().await?;
        Ok(response)
    }
}
