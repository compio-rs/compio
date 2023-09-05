use std::net::Ipv4Addr;

use compio::net::{TcpListener, TcpStream};

fn main() {
    compio::task::block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, (rx, _)) =
            futures_util::try_join!(TcpStream::connect(&addr), listener.accept()).unwrap();

        tx.send("Hello world!").await.0.unwrap();

        let buffer = Vec::with_capacity(64);
        let (n, buffer) = rx.recv(buffer).await;
        assert_eq!(n.unwrap(), buffer.len());
        println!("{}", String::from_utf8(buffer).unwrap());
    });
}
