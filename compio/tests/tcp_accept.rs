use compio::net::{IntoSocketAddrsStream, TcpListener, TcpStream};

async fn test_impl(addr: impl IntoSocketAddrsStream) {
    let listener = TcpListener::bind(addr).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = futures_channel::oneshot::channel();
    compio::runtime::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        assert!(tx.send(socket).is_ok());
    })
    .detach();
    let cli = TcpStream::connect(&addr).await.unwrap();
    let srv = rx.await.unwrap();
    assert_eq!(cli.local_addr().unwrap(), srv.peer_addr().unwrap());
}

macro_rules! test_accept {
    ($(($ident:ident, $target:expr),)*) => {
        $(
            #[compio_macros::test]
            async fn $ident() {
                println!("Testing {}...", stringify!($ident));
                test_impl($target).await;
            }
        )*
    };
}

test_accept! {
    (ip_str, "127.0.0.1:0"),
    (host_str, "localhost:0"),
    (socket_addr, "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap()),
    (str_port_tuple, ("127.0.0.1", 0)),
    (ip_port_tuple, ("127.0.0.1".parse::<std::net::IpAddr>().unwrap(), 0)),
}
