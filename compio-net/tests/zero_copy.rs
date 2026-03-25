use compio_buf::BufResult;
use compio_io::AsyncReadExt;
use compio_net::{TcpListener, TcpStream, UdpSocket};

#[compio_macros::test]
async fn tcp_zerocopy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let task = compio_runtime::spawn(async move { listener.accept().await.unwrap() });

    let tx = TcpStream::connect(&addr).await.unwrap();
    let (mut rx, _) = task.await.unwrap();

    let buffer = Vec::from(b"hello world" as &[u8]);
    let BufResult(res, fut) = tx.send_zerocopy(buffer).await;
    assert_eq!(res.unwrap(), 11);
    let buffer = fut.await;
    assert_eq!(buffer, b"hello world");

    let buf = Vec::with_capacity(11);
    let (_, buf) = rx.read_exact(buf).await.unwrap();
    assert_eq!(buf, b"hello world");
}

#[compio_macros::test]
async fn tcp_zerocopy_vectored() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let task = compio_runtime::spawn(async move { listener.accept().await.unwrap() });

    let tx = TcpStream::connect(&addr).await.unwrap();
    let (mut rx, _) = task.await.unwrap();

    let buffer = [
        Vec::from(b"hello" as &[u8]),
        Vec::from(b" " as &[u8]),
        Vec::from(b"world" as &[u8]),
    ];
    let BufResult(res, fut) = tx.send_zerocopy_vectored(buffer).await;
    assert_eq!(res.unwrap(), 11);
    let buffer = fut.await;
    assert_eq!(buffer[0], b"hello");
    assert_eq!(buffer[1], b" ");
    assert_eq!(buffer[2], b"world");

    let buf = Vec::with_capacity(11);
    let (_, buf) = rx.read_exact(buf).await.unwrap();
    assert_eq!(buf, b"hello world");
}

#[compio_macros::test]
async fn udp_zerocopy() {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = receiver.local_addr().unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let buffer = Vec::from(b"hello world" as &[u8]);
    let BufResult(res, fut) = sender.send_to_zerocopy(buffer, addr).await;
    assert_eq!(res.unwrap(), 11);
    let buffer = fut.await;
    assert_eq!(buffer, b"hello world");

    let (len, buf) = receiver.recv(Vec::with_capacity(11)).await.unwrap();
    assert_eq!(len, 11);
    assert_eq!(buf, b"hello world");
}

#[compio_macros::test]
async fn udp_zerocopy_vectored() {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = receiver.local_addr().unwrap();

    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let buffer = [
        Vec::from(b"hello" as &[u8]),
        Vec::from(b" " as &[u8]),
        Vec::from(b"world" as &[u8]),
    ];
    let BufResult(res, fut) = sender.send_to_zerocopy_vectored(buffer, addr).await;
    assert_eq!(res.unwrap(), 11);
    let buffer = fut.await;
    assert_eq!(buffer[0], b"hello");
    assert_eq!(buffer[1], b" ");
    assert_eq!(buffer[2], b"world");

    let (len, buf) = receiver.recv(Vec::with_capacity(11)).await.unwrap();
    assert_eq!(len, 11);
    assert_eq!(buf, b"hello world");
}
