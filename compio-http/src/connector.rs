use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use hyper::{service::Service, Uri};
use send_wrapper::SendWrapper;

use crate::HttpStream;

#[derive(Debug, Clone)]
pub struct Connector;

impl Service<Uri> for Connector {
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<Self::Response>> + Send>>;
    type Response = HttpStream;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Uri) -> Self::Future {
        Box::pin(SendWrapper::new(HttpStream::new(req)))
    }
}
