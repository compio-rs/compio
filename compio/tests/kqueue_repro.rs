use std::time::Duration;

use compio::io::{AsyncRead, AsyncWrite};
use compio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use compio_runtime::ResumeUnwind;

#[test]
fn tcp_roundtrip() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let buf = vec![0u8; 5];
            let compio::BufResult(res, buf) = AsyncRead::read(&mut &stream, buf).await;
            let n = res.unwrap();
            assert_eq!(&buf[..n], b"hello");
        });

        let client = TcpStream::connect(addr).await.unwrap();
        let compio::BufResult(res, _) = AsyncWrite::write(&mut &client, b"hello".to_vec()).await;
        res.unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("timed out")
            .resume_unwind()
            .unwrap();
    });
}

#[test]
fn ipc_roundtrip() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let path = std::env::temp_dir().join(format!(
            "compio-kq-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let listener = UnixListener::bind(&path).await.unwrap();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let buf = vec![0u8; 5];
            let compio::BufResult(res, buf) = AsyncRead::read(&mut &stream, buf).await;
            let n = res.unwrap();
            assert_eq!(&buf[..n], b"hello");
        });

        let client = UnixStream::connect(&path).await.unwrap();
        let compio::BufResult(res, _) = AsyncWrite::write(&mut &client, b"hello".to_vec()).await;
        res.unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("timed out")
            .resume_unwind()
            .unwrap();

        let _ = std::fs::remove_file(&path);
    });
}

#[test]
fn tcp_roundtrip_repeated() {
    for i in 0..20 {
        let rt = compio_runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();

            let server = compio_runtime::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let buf = vec![0u8; 16];
                let compio::BufResult(res, buf) = AsyncRead::read(&mut &stream, buf).await;
                let n = res.unwrap();
                String::from_utf8(buf[..n].to_vec()).unwrap()
            });

            let client = TcpStream::connect(addr).await.unwrap();
            let msg = format!("msg-{i}");
            let compio::BufResult(res, _) = AsyncWrite::write(&mut &client, msg.as_bytes().to_vec()).await;
            res.unwrap();

            let got = compio_runtime::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap_or_else(|_| panic!("iteration {i} timed out"))
                .resume_unwind()
                .unwrap();
            assert_eq!(got, msg);
        });
    }
}
