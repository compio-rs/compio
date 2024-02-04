use std::io::{Read, Write};

use compio_buf::BufResult;
use compio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::{TcpStream, UnixListener, UnixStream};

#[compio_macros::test]
async fn tcp_split() {
    const MSG: &[u8] = b"split";

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = compio_runtime::spawn_blocking(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(MSG).unwrap();

        let mut read_buf = [0u8; 32];
        let read_len = stream.read(&mut read_buf).unwrap();
        assert_eq!(&read_buf[..read_len], MSG);
    });

    let stream = TcpStream::connect(&addr).await.unwrap();
    let (mut read_half, mut write_half) = stream.into_split();

    let read_buf = [0u8; 32];
    let (read_res, buf) = read_half.read(read_buf).await.unwrap();
    assert_eq!(read_res, MSG.len());
    assert_eq!(&buf[..MSG.len()], MSG);

    write_half.write_all(MSG).await.unwrap();
    handle.await;
}

#[compio_macros::test]
async fn tcp_unsplit() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = compio_runtime::spawn_blocking(move || {
        drop(listener.accept().unwrap());
        drop(listener.accept().unwrap());
    });

    let stream1 = TcpStream::connect(&addr).await.unwrap();
    let (read1, write1) = stream1.into_split();

    let stream2 = TcpStream::connect(&addr).await.unwrap();
    let (_, write2) = stream2.into_split();

    let read1 = match read1.reunite(write2) {
        Ok(_) => panic!("Reunite should not succeed"),
        Err(err) => err.0,
    };

    read1.reunite(write1).expect("Reunite should succeed");

    handle.await;
}

#[compio_macros::test]
async fn unix_split() {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-split-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).unwrap();

    let client = UnixStream::connect(&sock_path).unwrap();
    let (server, _) = listener.accept().await.unwrap();

    let (mut a_read, mut a_write) = server.into_split();
    let (mut b_read, mut b_write) = client.into_split();

    let (a_response, b_response) = futures_util::future::try_join(
        send_recv_all(&mut a_read, &mut a_write, b"A"),
        send_recv_all(&mut b_read, &mut b_write, b"B"),
    )
    .await
    .unwrap();

    assert_eq!(a_response, b"B");
    assert_eq!(b_response, b"A");
}

async fn send_recv_all<R: AsyncRead, W: AsyncWrite>(
    read: &mut R,
    write: &mut W,
    input: &'static [u8],
) -> std::io::Result<Vec<u8>> {
    write.write_all(input).await.0?;
    write.shutdown().await?;

    let output = Vec::with_capacity(2);
    let BufResult(res, buf) = read.read_exact(output).await;
    assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
    Ok(buf)
}
