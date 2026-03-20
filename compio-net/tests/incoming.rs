use compio_io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream, UnixListener, UnixStream};
use compio_runtime::ResumeUnwind;
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
            i += 1;
            if i >= 2 {
                break;
            }
        }
    });

    for i in 0..2 {
        let mut client = TcpStream::connect(&addr).await.unwrap();
        let (_, text) = client.read_to_string(String::new()).await.unwrap();
        assert_eq!(text, format!("Hello, {}", i));
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
            i += 1;
            if i >= 2 {
                break;
            }
        }
    });

    for i in 0..2 {
        let mut client = UnixStream::connect(&sock_path).await.unwrap();
        let (_, text) = client.read_to_string(String::new()).await.unwrap();
        assert_eq!(text, format!("Hello, {}", i));
    }

    task.await.resume_unwind();
}
