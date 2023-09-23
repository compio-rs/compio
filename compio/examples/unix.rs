use compio::net::{UnixListener, UnixStream};
use tempfile::tempdir;

fn main() {
    compio::task::block_on(async {
        let dir = tempdir().unwrap();
        let path = dir.path().join("unix-example.sock");
        let listener = UnixListener::bind(&path).unwrap();

        let addr = listener.local_addr().unwrap();

        let tx = UnixStream::connect_addr(&addr).unwrap();
        let (rx, _) = listener.accept().await.unwrap();

        assert_eq!(addr, tx.peer_addr().unwrap());

        tx.send_all("Hello world!").await.0.unwrap();

        let buffer = Vec::with_capacity(12);
        let (n, buffer) = rx.recv_exact(buffer).await;
        n.unwrap();
        println!("{}", String::from_utf8(buffer).unwrap());
    });
}
