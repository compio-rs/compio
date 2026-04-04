use std::net::Ipv6Addr;

use compio_io::{AsyncReadManaged, AsyncReadMulti, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream, UdpSocket, UnixListener, UnixStream};
use futures_util::{StreamExt, TryStreamExt};

#[compio_macros::test]
async fn test_tcp_read_buffer_pool() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let mut stream = TcpStream::connect(addr).await.unwrap();

    assert_eq!(
        stream.read_managed(0).await.unwrap().unwrap().as_ref(),
        b"test"
    );
    let res = stream.read_managed(0).await;
    println!("{res:?}");
    assert!(matches!(res, Ok(None)));
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

    assert_eq!(
        connected.recv_managed(0).await.unwrap().unwrap().as_ref(),
        b"test"
    );
}

#[compio_macros::test]
async fn test_udp_recv_from_buffer_pool() {
    let listener = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let listener_addr = listener.local_addr().unwrap();
    let connected = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let connected_addr = connected.local_addr().unwrap();

    compio_runtime::spawn(async move {
        connected.send_to(b"test", listener_addr).await.unwrap();
    })
    .detach();

    let (buffer, addr) = listener.recv_from_managed(0).await.unwrap().unwrap();
    assert_eq!(buffer.as_ref(), b"test");
    assert_eq!(addr, connected_addr);
}

#[compio_macros::test]
async fn test_uds_recv_buffer_pool() {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-buffer-pool-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).await.unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let mut stream = UnixStream::connect(&sock_path).await.unwrap();

    assert_eq!(
        stream.read_managed(0).await.unwrap().unwrap().as_ref(),
        b"test"
    );
    assert!(matches!(stream.read_managed(0).await, Ok(None)));
}

#[compio_macros::test]
async fn test_tcp_recv_multi() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let mut stream = TcpStream::connect(addr).await.unwrap();

    let buffer = stream.read_multi(0).try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(buffer.len(), 1);
    assert_eq!(&*buffer[0], b"test");
}

#[compio_macros::test]
async fn test_udp_recv_multi() {
    let listener = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connected = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    connected.connect(addr).await.unwrap();
    let addr = connected.local_addr().unwrap();

    compio_runtime::spawn(async move {
        listener.send_to(b"test", addr).await.unwrap();
    })
    .detach();

    let buffer = connected.recv_multi(0).next().await.unwrap().unwrap();
    assert_eq!(&*buffer, b"test");
}

#[compio_macros::test]
async fn test_uds_recv_multi() {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-recv-multi-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).await.unwrap();

    compio_runtime::spawn(async move {
        let mut stream = listener.accept().await.unwrap().0;
        stream.write_all(b"test").await.unwrap();
    })
    .detach();

    let mut stream = UnixStream::connect(&sock_path).await.unwrap();

    let buffer = stream.read_multi(0).try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(buffer.len(), 1);
    assert_eq!(&*buffer[0], b"test");
}
