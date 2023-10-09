use std::num::NonZeroUsize;

use compio::{
    buf::IntoInner,
    dispatcher::Dispatcher,
    net::{TcpListener, TcpStream},
    runtime::{spawn, Unattached},
    BufResult,
};
use futures_util::{stream::FuturesUnordered, StreamExt};

#[compio::main(crate = "compio")]
async fn main() {
    const THREAD_NUM: usize = 5;
    const CLIENT_NUM: usize = 10;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(THREAD_NUM).unwrap())
        .build()
        .unwrap();
    let task = spawn(async move {
        let mut futures = FuturesUnordered::from_iter((0..CLIENT_NUM).map(|i| {
            let addr = &addr;
            async move {
                let cli = TcpStream::connect(addr).await.unwrap();
                cli.send_all(format!("Hello world {}!", i)).await.unwrap();
            }
        }));
        while let Some(()) = futures.next().await {}
    });
    for _i in 0..CLIENT_NUM {
        let (srv, _) = listener.accept().await.unwrap();
        let srv = Unattached::new(srv).unwrap();
        dispatcher
            .dispatch(move || {
                let srv = srv.into_inner();
                async move {
                    let BufResult(res, buf) = srv.recv(Vec::with_capacity(20)).await;
                    res?;
                    println!("{}", std::str::from_utf8(&buf).unwrap());
                    Ok(())
                }
            })
            .unwrap();
    }
    // Dispatcher::join is a blocking call, which may block the main thread. We need
    // to wait for the client first.
    task.await;
    for res in dispatcher.join() {
        res.unwrap();
    }
}
