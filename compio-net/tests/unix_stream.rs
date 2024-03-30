use compio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::{UnixListener, UnixStream};

#[compio_macros::test]
async fn accept_read_write() -> std::io::Result<()> {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).await?;

    let (mut client, (mut server, _)) =
        futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept()).unwrap();

    client.write_all("hello").await.0?;
    drop(client);

    let buf = Vec::with_capacity(5);
    let ((), buf) = server.read_exact(buf).await.unwrap();
    assert_eq!(&buf[..], b"hello");
    let len = server.read(buf).await.0?;
    assert_eq!(len, 0);
    Ok(())
}

#[compio_macros::test]
async fn shutdown() -> std::io::Result<()> {
    let dir = tempfile::Builder::new()
        .prefix("compio-uds-tests")
        .tempdir()
        .unwrap();
    let sock_path = dir.path().join("connect.sock");

    let listener = UnixListener::bind(&sock_path).await?;

    let (mut client, (mut server, _)) =
        futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept()).unwrap();

    // Shut down the client
    client.shutdown().await?;
    // Read from the server should return 0 to indicate the channel has been closed.
    let n = server.read(Vec::with_capacity(1)).await.0?;
    assert_eq!(n, 0);
    Ok(())
}
