use std::net::Ipv6Addr;

use compio_io::{AsyncReadManaged, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream, UdpSocket, UnixListener, UnixStream};
use compio_runtime::BufferPool;

#[compio_macros::test]
async fn test_tcp_read_buffer_pool() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let buffer_pool = BufferPool::new(1, 4).unwrap();
    let mut stream = TcpStream::connect(addr).await.unwrap();

    assert_eq!(
        stream.read_managed(&buffer_pool, 0).await.unwrap().as_ref(),
        b"test"
    );

    assert_eq!(
        stream
            .read_managed(&buffer_pool, 0)
            .await
            .map(|b| b.len())
            .unwrap_or_default(),
        0
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

    let buffer_pool = BufferPool::new(1, 4).unwrap();

    assert_eq!(
        connected
            .recv_managed(&buffer_pool, 0)
            .await
            .unwrap()
            .as_ref(),
        b"test"
    );
}

#[compio_macros::test]
async fn test_uds_recv_buffer_pool() {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-buffer-pool-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).await.unwrap();

    let (mut client, (mut server, _)) =
        futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept()).unwrap();

    client.write_all("test").await.unwrap();
    drop(client);

    let buffer_pool = BufferPool::new(1, 4).unwrap();

    assert_eq!(
        server.read_managed(&buffer_pool, 0).await.unwrap().as_ref(),
        b"test"
    );

    assert_eq!(
        server
            .read_managed(&buffer_pool, 0)
            .await
            .map(|b| b.len())
            .unwrap_or_default(),
        0
    );
}
