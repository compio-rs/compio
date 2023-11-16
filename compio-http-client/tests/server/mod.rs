use std::{
    convert::Infallible,
    future::Future,
    net::{Ipv4Addr, SocketAddr},
};

use compio_http::HyperStream;
use compio_net::TcpListener;
use futures_channel::oneshot;
use hyper::body::Incoming;

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
    F: Fn(http::Request<Incoming>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = http::Response<String>> + Send + 'static,
{
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let listener = TcpListener::bind(&(Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = async move {
        while let Ok(None) = shutdown_rx.try_recv() {
            let (stream, _) = listener.accept().await.unwrap();
            hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    HyperStream::new(stream),
                    hyper::service::service_fn({
                        let func = func.clone();
                        move |req| {
                            let fut = func(req);
                            async move { Ok::<_, Infallible>(fut.await) }
                        }
                    }),
                )
                .await
                .unwrap();
        }
    };

    compio_runtime::spawn(srv).detach();

    Server {
        addr,
        shutdown_tx: Some(shutdown_tx),
    }
}
