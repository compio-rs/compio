use compio_buf::BufResult;
use compio_io::{AsyncRead, AsyncReadExt, AsyncZeroCopyVectoredWrite, AsyncZeroCopyWrite};
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
async fn tcp_owned_write_half_zerocopy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let task = compio_runtime::spawn(async move { listener.accept().await.unwrap() });

    let tx = TcpStream::connect(&addr).await.unwrap();
    let (_tx_read, mut tx_write) = tx.into_split();

    let (mut rx, _) = task.await.unwrap();

    let buffer = Vec::from(b"hello world" as &[u8]);
    let BufResult(res, fut) = tx_write.write_zerocopy(buffer).await;
    assert_eq!(res.unwrap(), 11);
    let buffer = fut.await;
    assert_eq!(buffer, b"hello world");

    let buf = Vec::with_capacity(11);
    let (_, buf) = rx.read_exact(buf).await.unwrap();
    assert_eq!(buf, b"hello world");
}

#[compio_macros::test]
async fn tcp_owned_write_half_zerocopy_all() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn receiver — reads slowly, creating backpressure
    let recv_handle = compio_runtime::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Set tiny recv buffer to create backpressure
        #[cfg(unix)]
        {
            set_sock_buf(&stream, 4096, 4096);
        }
        let mut received = Vec::new();

        let mut buf = Vec::with_capacity(1024);
        loop {
            let (res, next_buf) = stream.read(buf).await.unwrap();
            if res == 0 {
                break;
            }

            buf = next_buf;
            received.extend_from_slice(&buf[..res]);
        }
        received
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    // Tiny send buffer forces partial sends
    #[cfg(unix)]
    {
        set_sock_buf(&stream, 4096, 4096);
    }

    let (reader, mut writer) = stream.into_split();

    // Send 256KB of data
    let data: Vec<u8> = (0u8..=255).cycle().take(1 << 18).collect(); // 256 KB
    let BufResult(res, fut) = writer.write_zerocopy_all(data).await;
    res.unwrap();
    let buffer = fut.await;

    // Signal EOF
    drop(writer);
    drop(reader);

    let received = recv_handle.await.unwrap();
    assert_eq!(received.len(), buffer.len(), "all bytes must be received");
    assert_eq!(received, buffer, "all bytes must arrive in order");
}

#[compio_macros::test]
async fn tcp_owned_write_half_zerocopy_vectored() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let task = compio_runtime::spawn(async move { listener.accept().await.unwrap() });

    let tx = TcpStream::connect(&addr).await.unwrap();
    let (_tx_read, mut tx_write) = tx.into_split();

    let (mut rx, _) = task.await.unwrap();

    let buffer = [
        Vec::from(b"hello" as &[u8]),
        Vec::from(b" " as &[u8]),
        Vec::from(b"world" as &[u8]),
    ];
    let BufResult(res, _) = tx_write.write_zerocopy_vectored(buffer).await;
    assert_eq!(res.unwrap(), 11);

    let buf = Vec::with_capacity(11);
    let (_, buf) = rx.read_exact(buf).await.unwrap();
    assert_eq!(buf, b"hello world");
}

#[compio_macros::test]
async fn tcp_owned_write_half_zerocopy_vectored_all() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn receiver — reads slowly, creating backpressure
    let recv_handle = compio_runtime::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Set tiny recv buffer to create backpressure
        #[cfg(unix)]
        {
            set_sock_buf(&stream, 4096, 4096);
        }
        let mut received = Vec::new();

        let mut buf = Vec::with_capacity(1024);
        loop {
            let (res, next_buf) = stream.read(buf).await.unwrap();
            if res == 0 {
                break;
            }

            buf = next_buf;
            received.extend_from_slice(&buf[..res]);
        }
        received
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    // Tiny send buffer forces partial sends
    #[cfg(unix)]
    {
        set_sock_buf(&stream, 4096, 4096);
    }

    let (reader, mut writer) = stream.into_split();

    // Send 256KB of data
    let data: Vec<u8> = (0u8..=255).cycle().take(1 << 18).collect(); // 256 KB
    let data_vector: Vec<Vec<u8>> = data.chunks(4096).map(Vec::from).collect(); // 4096 bytes per chunk
    let BufResult(res, fut) = writer.write_zerocopy_vectored_all(data_vector).await;
    res.unwrap();
    let _ = fut.await;

    // Signal EOF
    drop(writer);
    drop(reader);

    let received = recv_handle.await.unwrap();
    assert_eq!(received.len(), data.len(), "all bytes must be received");
    assert_eq!(received, data, "all bytes must arrive in order");
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

// Helper utility to set the socket buffer size
#[cfg(unix)]
fn set_sock_buf(stream: &TcpStream, send: usize, recv: usize) {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    unsafe {
        let v = send as libc::c_int;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &v as *const _ as _,
            std::mem::size_of_val(&v) as _,
        );
        let v = recv as libc::c_int;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &v as *const _ as _,
            std::mem::size_of_val(&v) as _,
        );
    }
}
