use std::net::Ipv6Addr;

use compio_io::AsyncWriteExt;
use compio_net::{TcpListener, TcpStream, UdpSocket};
use compio_runtime::buffer_pool::BufferPool;

#[compio_macros::test]
async fn test_tcp_read_buffer_pool() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let buffer_pool = BufferPool::new(2, 4).unwrap();
    let stream = TcpStream::connect(addr).await.unwrap();

    assert_eq!(
        stream
            .recv_buffer_pool(&buffer_pool, 0)
            .await
            .unwrap()
            .as_ref(),
        b"test"
    );

    assert!(
        stream
            .recv_buffer_pool(&buffer_pool, 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[compio_macros::test]
async fn test_udp_read_buffer_pool() {
    let listener = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connected = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    connected.connect(addr).await.unwrap();
    let addr = connected.local_addr().unwrap();

    compio_runtime::spawn(async move {
        listener.send_to(b"test", addr).await.unwrap();
    })
    .detach();

    let buffer_pool = BufferPool::new(2, 4).unwrap();

    assert_eq!(
        connected
            .recv_buffer_pool(&buffer_pool, 0)
            .await
            .unwrap()
            .as_ref(),
        b"test"
    );
}
