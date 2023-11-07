use std::{
    convert::Infallible,
    future::Future,
    net::{Ipv4Addr, SocketAddr},
};

use compio_http::{Acceptor, CompioExecutor};
use compio_net::TcpListener;
use futures_channel::oneshot;

pub struct Server {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Server {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub async fn http<F, Fut>(func: F) -> Server
where
    F: Fn(http::Request<hyper::Body>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = http::Response<hyper::Body>> + Send + 'static,
{
    let listener = TcpListener::bind(&(Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let acceptor = Acceptor::from_listener(listener);
    let srv = hyper::Server::builder(acceptor)
        .executor(CompioExecutor)
        .serve(hyper::service::make_service_fn(move |_| {
            let func = func.clone();
            async move {
                Ok::<_, Infallible>(hyper::service::service_fn(move |req| {
                    let fut = func(req);
                    async move { Ok::<_, Infallible>(fut.await) }
                }))
            }
        }));

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let srv = srv.with_graceful_shutdown(async move {
        let _ = shutdown_rx.await;
    });

    compio_runtime::spawn(srv).detach();

    Server {
        addr,
        shutdown_tx: Some(shutdown_tx),
    }
}
