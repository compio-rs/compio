use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use hyper::{rt::Executor, service::Service, Uri};
use send_wrapper::SendWrapper;

use crate::{HttpStream, TlsBackend};

#[derive(Debug, Clone)]
pub struct CompioExecutor;

impl Executor<Pin<Box<dyn Future<Output = ()> + Send>>> for CompioExecutor {
    fn execute(&self, fut: Pin<Box<dyn Future<Output = ()> + Send>>) {
        compio_runtime::spawn(fut).detach()
    }
}

#[derive(Debug, Clone)]
pub struct Connector {
    tls: TlsBackend,
}

impl Connector {
    pub fn new(tls: TlsBackend) -> Self {
        Self { tls }
    }
}

impl Service<Uri> for Connector {
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<Self::Response>> + Send>>;
    type Response = HttpStream;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Uri) -> Self::Future {
        Box::pin(SendWrapper::new(HttpStream::new(req, self.tls)))
    }
}
