#![cfg(windows)]

use compio_io::AsyncWrite;
use compio_net::{TcpListener, TcpStream};
use compio_runtime::ResumeUnwind;

#[test]
fn disconnect() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = compio_runtime::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let socket = socket.disconnect().await.unwrap();
            let (mut socket, _) = listener.accept_with(socket).await.unwrap();
            socket.shutdown().await.unwrap();
            socket.close().await.unwrap();
        });

        for _i in 0..2 {
            let mut client = TcpStream::connect(addr).await.unwrap();
            client.shutdown().await.unwrap();
            client.close().await.unwrap();
        }

        task.await.resume_unwind().expect("shouldn't be cancelled");
    })
}

#[test]
fn reuse() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr1 = listener1.local_addr().unwrap();
        let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr2 = listener2.local_addr().unwrap();

        let task = compio_runtime::spawn(async move {
            let (socket, _) = listener1.accept().await.unwrap();
            let socket = socket.disconnect().await.unwrap();
            let (mut socket, _) = listener2.accept_with(socket).await.unwrap();
            socket.shutdown().await.unwrap();
            socket.close().await.unwrap();
        });

        let client = TcpStream::connect(addr1).await.unwrap();
        let client = client.disconnect().await.unwrap();
        let mut client = client.connect(addr2).await.unwrap();
        client.shutdown().await.unwrap();
        client.close().await.unwrap();

        task.await.resume_unwind().expect("shouldn't be cancelled");
    })
}
