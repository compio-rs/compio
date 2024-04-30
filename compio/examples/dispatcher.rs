use std::num::NonZeroUsize;

use compio::{
    dispatcher::Dispatcher,
    io::{AsyncRead, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    runtime::spawn,
    BufResult,
};
use futures_util::{stream::FuturesUnordered, StreamExt};

#[compio::main]
async fn main() {
    const THREAD_NUM: usize = 5;
    const CLIENT_NUM: usize = 10;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(THREAD_NUM).unwrap())
        .build()
        .unwrap();
    spawn(async move {
        let mut futures = FuturesUnordered::from_iter((0..CLIENT_NUM).map(|i| {
            let addr = &addr;
            async move {
                let mut cli = TcpStream::connect(addr).await.unwrap();
                cli.write_all(format!("Hello world {}!", i)).await.unwrap();
            }
        }));
        while let Some(()) = futures.next().await {}
    })
    .detach();
    let mut handles = FuturesUnordered::new();
    for _i in 0..CLIENT_NUM {
        let (mut srv, _) = listener.accept().await.unwrap();
        let handle = dispatcher
            .execute(move || async move {
                let BufResult(res, buf) = srv.read(Vec::with_capacity(20)).await;
                res.unwrap();
                println!("{}", std::str::from_utf8(&buf).unwrap());
            })
            .unwrap();
        handles.push(handle.join());
    }
    while handles.next().await.is_some() {}
    dispatcher.join().await.unwrap();
}
