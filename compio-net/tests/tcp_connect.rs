#![cfg_attr(feature = "sanitize", feature(cfg_sanitize))]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use compio_net::{TcpListener, TcpStream, ToSocketAddrsAsync};
use compio_runtime::ResumeUnwind;

async fn test_connect_ip_impl(
    target: impl ToSocketAddrsAsync,
    assert_fn: impl FnOnce(&SocketAddr) -> bool,
) {
    let listener = TcpListener::bind(target).await.unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(assert_fn(&addr));

    let task = compio_runtime::spawn(async move {
        let (socket, addr) = listener.accept().await.unwrap();
        assert_eq!(addr, socket.peer_addr().unwrap());
        socket
    });

    let mine = TcpStream::connect(&addr).await.unwrap();
    let theirs = task.await.resume_unwind().expect("shouldn't be canceled");

    assert_eq!(mine.local_addr().unwrap(), theirs.peer_addr().unwrap());
    assert_eq!(theirs.local_addr().unwrap(), mine.peer_addr().unwrap());
}

macro_rules! test_connect_ip {
    ($(($ident:ident, $target:expr, $addr_f:path),)*) => {
        $(
            #[compio_macros::test]
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

async fn test_bind_and_connect_ip_impl(
    bind_addr: SocketAddr,
    target: impl ToSocketAddrsAsync,
    assert_fn: impl FnOnce(&SocketAddr) -> bool,
) {
    let listener = TcpListener::bind(target).await.unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(assert_fn(&addr));

    let task = compio_runtime::spawn(async move {
        let (socket, addr) = listener.accept().await.unwrap();
        assert_eq!(addr, socket.peer_addr().unwrap());
        socket
    });

    let mine = TcpStream::bind_and_connect(bind_addr, &addr).await.unwrap();
    let theirs = task.await.resume_unwind().expect("shouldn't be canceled");

    assert_eq!(mine.local_addr().unwrap(), theirs.peer_addr().unwrap());
    assert_eq!(theirs.local_addr().unwrap(), mine.peer_addr().unwrap());
}

macro_rules! test_bind_and_connect_ip {
    ($(($ident:ident, $bind_addr:expr, $target:expr, $addr_f:path),)*) => {
        $(
            #[compio_macros::test]
            async fn $ident() {
                test_bind_and_connect_ip_impl($bind_addr, $target, $addr_f).await;
            }
        )*
    }
}

test_bind_and_connect_ip! {
    (bind_and_connect_v4, SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0), "127.0.0.1:0", SocketAddr::is_ipv4),
    (bind_and_connect_v6, SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 0), "[::1]:0", SocketAddr::is_ipv6),
}

async fn test_connect_impl<A: ToSocketAddrsAsync>(mapping: impl FnOnce(&TcpListener) -> A) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = mapping(&listener);
    let server = async {
        listener.accept().await.unwrap();
    };

    let client = async {
        match TcpStream::connect(addr).await {
            Ok(_) => (),
            Err(e) => panic!("Failed to connect: {e}"),
        }
    };

    futures_util::join!(server, client);
}

macro_rules! test_connect {
    ($(($(#[$m:meta])* $ident:ident, $mapping:tt),)*) => {
        $(
            #[compio_macros::test]
            $(#[$m])*
            async fn $ident() {
                #[allow(unused_parens)]
                test_connect_impl($mapping).await;
            }
        )*
    }
}

test_connect! {
    (ip_string, (|listener: &TcpListener| {
        format!("127.0.0.1:{}", listener.local_addr().unwrap().port())
    })),
    (#[cfg_attr(feature = "sanitize", cfg(not(sanitize = "address")))] ip_str, (|listener: &TcpListener| {
        let s = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
        let slice: &str = &*Box::leak(s.into_boxed_str());
        slice
    })),
    (ip_port_tuple, (|listener: &TcpListener| {
        let addr = listener.local_addr().unwrap();
        (addr.ip(), addr.port())
    })),
    (#[cfg_attr(feature = "sanitize", cfg(not(sanitize = "address")))] ip_port_tuple_ref, (|listener: &TcpListener| {
        let addr = listener.local_addr().unwrap();
        let tuple_ref: &(IpAddr, u16) = &*Box::leak(Box::new((addr.ip(), addr.port())));
        tuple_ref
    })),
    (ip_str_port_tuple, (|listener: &TcpListener| {
        let addr = listener.local_addr().unwrap();
        ("127.0.0.1", addr.port())
    })),
}

#[compio_macros::test]
async fn connect_invalid_dst() {
    assert!(TcpStream::connect("127.0.0.0:0").await.is_err());
}
