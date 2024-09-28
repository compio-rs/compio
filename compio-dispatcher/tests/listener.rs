use std::num::NonZeroUsize;

use compio_buf::arrayvec::ArrayVec;
use compio_dispatcher::Dispatcher;
use compio_io::{AsyncReadExt, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream};
use compio_runtime::spawn;
use futures_util::{StreamExt, stream::FuturesUnordered};

#[compio_macros::test]
async fn listener_dispatch() {
    const THREAD_NUM: usize = 5;
    const CLIENT_NUM: usize = 10;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(THREAD_NUM).unwrap())
        .build()
        .unwrap();
    let task = spawn(async move {
        let mut futures = FuturesUnordered::from_iter((0..CLIENT_NUM).map(|_| async {
            let mut cli = TcpStream::connect(&addr).await.unwrap();
            cli.write_all("Hello world!").await.unwrap();
        }));
        while let Some(()) = futures.next().await {}
    });
    let mut handles = FuturesUnordered::new();
    for _i in 0..CLIENT_NUM {
        let (mut srv, _) = listener.accept().await.unwrap();
        let handle = dispatcher
            .dispatch(move || async move {
                let (_, buf) = srv.read_exact(ArrayVec::<u8, 12>::new()).await.unwrap();
                assert_eq!(buf.as_slice(), b"Hello world!");
            })
            .unwrap();
        handles.push(handle);
    }
    while handles.next().await.is_some() {}
    let (_, results) = futures_util::join!(task, dispatcher.join());
    results.unwrap();
}
