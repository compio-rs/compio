use std::{
    convert::Infallible,
    future::Future,
    net::{self, Ipv4Addr},
    sync::mpsc as std_mpsc,
    thread,
    time::Duration,
};

use tokio::{runtime, sync::oneshot};

pub struct Server {
    addr: net::SocketAddr,
    panic_rx: std_mpsc::Receiver<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Server {
    pub fn addr(&self) -> net::SocketAddr {
        self.addr
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if !::std::thread::panicking() {
            self.panic_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("test server should not panic");
        }
    }
}

pub fn http<F, Fut>(func: F) -> Server
where
    F: Fn(http::Request<hyper::Body>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = http::Response<hyper::Body>> + Send + 'static,
{
    // Spawn new runtime in thread to prevent reactor execution context conflict
    thread::spawn(move || {
        let rt = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("new rt");
        #[allow(clippy::async_yields_async)]
        let srv = rt.block_on(async move {
            hyper::Server::bind(&(Ipv4Addr::LOCALHOST, 0).into()).serve(
                hyper::service::make_service_fn(move |_| {
                    let func = func.clone();
                    async move {
                        Ok::<_, Infallible>(hyper::service::service_fn(move |req| {
                            let fut = func(req);
                            async move { Ok::<_, Infallible>(fut.await) }
                        }))
                    }
                }),
            )
        });

        let addr = srv.local_addr();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let srv = srv.with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });

        let (panic_tx, panic_rx) = std_mpsc::channel();
        let tname = format!(
            "test({})-support-server",
            thread::current().name().unwrap_or("<unknown>")
        );
        thread::Builder::new()
            .name(tname)
            .spawn(move || {
                rt.block_on(srv).unwrap();
                let _ = panic_tx.send(());
            })
            .expect("thread spawn");

        Server {
            addr,
            panic_rx,
            shutdown_tx: Some(shutdown_tx),
        }
    })
    .join()
    .unwrap()
}