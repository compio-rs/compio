use std::net::Ipv6Addr;

use compio_io::AsyncWriteExt;
use compio_net::{TcpListener, TcpStream};

#[compio_macros::test]
async fn test_tcp_read_buffer_pool() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let buffer_pool = compio_runtime::create_buffer_pool(2, 4).await.unwrap();
    let stream = TcpStream::connect(addr).await.unwrap();

    assert_eq!(
        stream
            .recv_buffer_pool(&buffer_pool, 0)
            .await
            .unwrap()
            .as_ref(),
        b"test"
    );
}
