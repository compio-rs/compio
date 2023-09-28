use std::net::{IpAddr, SocketAddr};

use compio::net::{TcpListener, TcpStream, ToSockAddrs};

async fn test_connect_ip_impl(
    target: impl ToSockAddrs,
    assert_fn: impl FnOnce(&SocketAddr) -> bool,
) {
    let listener = TcpListener::bind(target).unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(assert_fn(&addr.as_socket().unwrap()));

    let (tx, rx) = futures_channel::oneshot::channel();

    compio::task::spawn(async move {
        let (socket, addr) = listener.accept().await.unwrap();
        assert_eq!(addr, socket.peer_addr().unwrap());
        assert!(tx.send(socket).is_ok());
    })
    .detach();

    let mine = TcpStream::connect(&addr).await.unwrap();
    let theirs = rx.await.unwrap();

    assert_eq!(mine.local_addr().unwrap(), theirs.peer_addr().unwrap());
    assert_eq!(theirs.local_addr().unwrap(), mine.peer_addr().unwrap());
}

macro_rules! test_connect_ip {
    ($(($ident:ident, $target:expr, $addr_f:path),)*) => {
        $(
            #[compio::test]
            async fn $ident() {
                test_connect_ip_impl($target, $addr_f).await;
            }
        )*
    }
}

test_connect_ip! {
    (connect_v4, "127.0.0.1:0", SocketAddr::is_ipv4),
    (connect_v6, "[::1]:0", SocketAddr::is_ipv6),
}

async fn test_connect_impl<A: ToSockAddrs>(mapping: impl FnOnce(&TcpListener) -> A) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = mapping(&listener);
    let server = async {
        listener.accept().await.unwrap();
    };

    let client = async {
        match TcpStream::connect(addr).await {
            Ok(_) => (),
            Err(e) => panic!("Failed to connect: {}", e),
        }
    };

    futures_util::join!(server, client);
}

macro_rules! test_connect {
    ($(($ident:ident, $mapping:tt),)*) => {
        $(
            #[compio::test]
            async fn $ident() {
                #[allow(unused_parens)]
                test_connect_impl($mapping).await;
            }
        )*
    }
}

test_connect! {
    (ip_string, (|listener: &TcpListener| {
        format!("127.0.0.1:{}", listener.local_addr().unwrap().as_socket().unwrap().port())
    })),
    (ip_str, (|listener: &TcpListener| {
        let s = format!("127.0.0.1:{}", listener.local_addr().unwrap().as_socket().unwrap().port());
        let slice: &str = &*Box::leak(s.into_boxed_str());
        slice
    })),
    (ip_port_tuple, (|listener: &TcpListener| {
        let addr = listener.local_addr().unwrap().as_socket().unwrap();
        (addr.ip(), addr.port())
    })),
    (ip_port_tuple_ref, (|listener: &TcpListener| {
        let addr = listener.local_addr().unwrap().as_socket().unwrap();
        let tuple_ref: &(IpAddr, u16) = &*Box::leak(Box::new((addr.ip(), addr.port())));
        tuple_ref
    })),
    (ip_str_port_tuple, (|listener: &TcpListener| {
        let addr = listener.local_addr().unwrap().as_socket().unwrap();
        ("127.0.0.1", addr.port())
    })),
}

#[compio::test]
async fn connect_invalid_dst() {
    assert!(TcpStream::connect("127.0.0.1:1").await.is_err());
}
