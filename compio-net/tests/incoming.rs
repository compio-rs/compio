use std::time::Duration;

use compio_buf::BufResult;
use compio_io::{AsyncRead, AsyncReadExt, AsyncReadMulti as _, AsyncWrite, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream, UnixListener, UnixStream};
use compio_runtime::{CancelToken, ResumeUnwind, StreamExt as _, time::timeout};
use futures_util::StreamExt;

#[compio_macros::test]
async fn incoming_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = compio_runtime::spawn(async move {
        let mut incoming = listener.incoming();
        let mut i = 0usize;
        while let Some(stream) = incoming.next().await {
            let mut stream = stream.unwrap();
            stream.write_all(format!("Hello, {}", i)).await.unwrap();
            stream.shutdown().await.unwrap();
            stream.read_exact([0u8; 8]).await.unwrap();
            i += 1;
            if i >= 2 {
                break;
            }
        }
    });

    for i in 0..2 {
        let mut client = TcpStream::connect(&addr).await.unwrap();
        let (_, text) = client.peek([0u8; 8]).await.unwrap();
        assert_eq!(text, format!("Hello, {}", i).as_bytes());
        let (_, text) = client.read_exact([0u8; 8]).await.unwrap();
        assert_eq!(text, format!("Hello, {}", i).as_bytes());
        client.write_all(text).await.unwrap();
        client.shutdown().await.unwrap();
        client.read([0u8; 1]).await.unwrap(); 
    }

    task.await.resume_unwind();
}

#[compio_macros::test]
async fn incoming_unix() {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-incoming-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).await.unwrap();
    let task = compio_runtime::spawn(async move {
        let mut incoming = listener.incoming();
        let mut i = 0usize;
        while let Some(stream) = incoming.next().await {
            let mut stream = stream.unwrap();
            stream.write_all(format!("Hello, {}", i)).await.unwrap();
            stream.shutdown().await.unwrap();
            stream.read_exact([0u8; 8]).await.unwrap();
            i += 1;
            if i >= 2 {
                break;
            }
        }
    });

    for i in 0..2 {
        let mut client = UnixStream::connect(&sock_path).await.unwrap();
        let (_, text) = client.read_exact([0u8; 8]).await.unwrap();
        assert_eq!(text, format!("Hello, {}", i).as_bytes());
        client.write_all(text).await.unwrap();
        client.shutdown().await.unwrap();
        client.read([0u8; 1]).await.unwrap();
    }

    task.await.resume_unwind();
}

#[compio_macros::test]
async fn incoming_tcp_multi_cancel() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();

    let addr = listener.local_addr().unwrap();

    compio_runtime::spawn(async move {
        let mut tx = TcpStream::connect(addr).await.unwrap();

        let mut v = vec![0xAA, 128];
        loop {
            let BufResult(res, buf) = tx.write(v).await;
            res.unwrap();
            v = buf;
        }
    })
    .detach();

    let (mut rx, _) = listener.accept().await.unwrap();

    let ct = CancelToken::new();
    let mut s = rx.read_multi(0).with_cancel(ct.clone());

    assert!(s.next().await.is_some_and(|r| r.is_ok()));

    ct.cancel();

    timeout(Duration::from_millis(200), async move {
        while let Some(Ok(_)) = s.next().await {}
    })
    .await
    .unwrap();
}
