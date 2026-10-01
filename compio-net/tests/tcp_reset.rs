//! A peer that resets the connection (RST) must be reported as an error, not as
//! a clean EOF (`Ok(0)`).

use std::{
    io::{ErrorKind, Write},
    net::{SocketAddr, TcpStream as StdTcpStream},
    time::Duration,
};

use compio_io::{AsyncRead, AsyncReadExt};
use compio_net::TcpListener;
use socket2::SockRef;

const MSG: &[u8] = b"compio";

/// Connect to `addr`, send [`MSG`], then abort the connection with an RST.
fn reset_peer(addr: SocketAddr, delay: Duration) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut stream = StdTcpStream::connect(addr).unwrap();
        stream.write_all(MSG).unwrap();
        stream.flush().unwrap();
        // Give the peer time to receive `MSG` and start waiting for more, so
        // that the RST completes a pending read instead of an idle socket.
        std::thread::sleep(delay);
        // A zero linger makes `closesocket` send an RST instead of a FIN.
        SockRef::from(&stream)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(stream);
    })
}

#[compio_macros::test]
async fn tcp_read_after_reset() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = reset_peer(addr, Duration::from_millis(200));

    let mut stream = listener.accept().await.unwrap().0;

    stream
        .read_exact(Vec::with_capacity(MSG.len()))
        .await
        .unwrap();

    // Pending while the peer resets: this must not look like a clean EOF.
    let res = stream.read(Vec::with_capacity(8)).await.0;
    let err = res.expect_err("a reset connection must not be reported as EOF");
    assert_eq!(err.kind(), ErrorKind::ConnectionReset, "{err:?}");

    peer.join().unwrap();
}
