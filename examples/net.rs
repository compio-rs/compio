use compio::net::{TcpListener, TcpStream};
use socket2::SockAddr;
use std::net::{Ipv4Addr, SocketAddrV4};

fn main() {
    compio::task::block_on(async {
        let listener =
            TcpListener::bind(SockAddr::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).unwrap();
        let addr = listener.local_addr().unwrap();
        println!("{}", addr.as_socket().unwrap());

        let (tx, (rx, addr)) =
            futures_util::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();

        assert_eq!(addr, tx.local_addr().unwrap());
        assert_eq!(addr, rx.peer_addr().unwrap());

        tx.send("Hello world!").await.0.unwrap();

        let buffer = Vec::with_capacity(64);
        let (n, buffer) = rx.recv(buffer).await;
        assert_eq!(n.unwrap(), buffer.len());
        println!("{:?}", buffer);
    });
}
