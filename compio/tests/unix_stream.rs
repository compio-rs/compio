use std::net::Shutdown;

use compio::net::{UnixListener, UnixStream};

#[compio::test]
async fn accept_read_write() -> std::io::Result<()> {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path)?;

    let client = UnixStream::connect(&sock_path)?;
    let (server, _) = listener.accept().await?;

    let write_len = client.send_all("hello").await.0?;
    assert_eq!(write_len, 5);
    drop(client);

    let buf = Vec::with_capacity(5);
    let (res, buf) = server.recv_exact(buf).await;
    assert_eq!(res.unwrap(), 5);
    assert_eq!(&buf[..], b"hello");
    let len = server.recv(buf).await.0?;
    assert_eq!(len, 0);
    Ok(())
}

#[compio::test]
async fn shutdown() -> std::io::Result<()> {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path)?;

    let client = UnixStream::connect(&sock_path)?;
    let (server, _) = listener.accept().await?;

    // Shut down the client
    client.shutdown(Shutdown::Both)?;
    // Read from the server should return 0 to indicate the channel has been closed.
    let n = server.recv(Vec::with_capacity(1)).await.0?;
    assert_eq!(n, 0);
    Ok(())
}
