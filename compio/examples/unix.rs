use compio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};
use tempfile::tempdir;

#[compio::main]
async fn main() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("unix-example.sock");
    let listener = UnixListener::bind(&path).unwrap();

    let addr = listener.local_addr().unwrap();

    let (mut tx, (mut rx, _)) =
        futures_util::try_join!(UnixStream::connect_addr(&addr), listener.accept()).unwrap();

    assert_eq!(addr, tx.peer_addr().unwrap());

    tx.write_all("Hello world!").await.0.unwrap();

    let buffer = Vec::with_capacity(12);
    let (_, buffer) = rx.read_exact(buffer).await.unwrap();
    println!("{}", String::from_utf8(buffer).unwrap());
}
