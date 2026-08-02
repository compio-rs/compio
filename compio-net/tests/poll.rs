use std::{
    io::{self, IoSlice},
    net::{Ipv4Addr, SocketAddrV4},
    pin::Pin,
};

use compio_runtime::fd::PollFd;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

fn is_would_block(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(libc::EINPROGRESS)
    }
    #[cfg(not(unix))]
    {
        e.kind() == io::ErrorKind::WouldBlock
    }
}

#[compio_macros::test]
async fn poll_connect() {
    let listener = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
        .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
        .unwrap();
    listener.listen(4).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = PollFd::new(listener).unwrap();
    let accept_task = async {
        std::future::poll_fn(|cx| listener.poll_accept_with(cx, |listener| listener.accept()))
            .await
            .unwrap()
    };

    let client = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    client.set_nonblocking(true).unwrap();
    let client = PollFd::new(client).unwrap();
    let connect_task = async {
        match client.connect(&addr) {
            Ok(_) => Ok(()),
            Err(e) if is_would_block(&e) => client.connect_ready().await,
            Err(e) => Err(e),
        }
    };
    let ((tx, _), res) = futures_util::join!(accept_task, connect_task);
    res.unwrap();

    tx.set_nonblocking(true).unwrap();
    let mut tx = PollFd::new(tx).unwrap();

    let send_task = async {
        futures_util::AsyncWriteExt::write(&mut tx, b"Hello world!")
            .await
            .unwrap()
    };

    let mut buffer = Vec::with_capacity(12);
    let recv_task = async {
        std::future::poll_fn(|cx| {
            client.poll_read_with(cx, |client| {
                let n = client.recv(buffer.spare_capacity_mut())?;
                unsafe { buffer.set_len(n) };
                Ok(n)
            })
        })
        .await
        .unwrap()
    };

    let (write, read) = futures_util::join!(send_task, recv_task);
    assert_eq!(write, 12);
    assert_eq!(read, 12);
    assert_eq!(buffer, b"Hello world!");
}

#[compio_macros::test]
async fn poll_write_vectored() {
    let (mut tx, rx) = connected_pair();

    // Exercise the `AsyncWrite` impl rather than the inherent method: the trait's
    // default only writes the first non-empty buffer.
    let bufs = [
        IoSlice::new(b"Hello"),
        IoSlice::new(b" "),
        IoSlice::new(b"world!"),
    ];
    let written = std::future::poll_fn(|cx| {
        futures_util::AsyncWrite::poll_write_vectored(Pin::new(&mut tx), cx, &bufs)
    })
    .await
    .unwrap();
    // A short write is allowed, but anything past the first buffer can only
    // come from a vectored write.
    assert!(
        written > 5,
        "expected a writev covering more than the first buffer, wrote {written}"
    );

    let mut buffer = Vec::with_capacity(written);
    let read = std::future::poll_fn(|cx| {
        rx.poll_read_with(cx, |rx| {
            let n = rx.recv(buffer.spare_capacity_mut())?;
            unsafe { buffer.set_len(n) };
            Ok(n)
        })
    })
    .await
    .unwrap();

    assert_eq!(read, written);
    assert_eq!(buffer, &b"Hello world!"[..written]);
}

fn connected_pair() -> (PollFd<Socket>, PollFd<Socket>) {
    let listener = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    listener
        .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
        .unwrap();
    listener.listen(1).unwrap();

    let client = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    client.connect(&listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();

    client.set_nonblocking(true).unwrap();
    server.set_nonblocking(true).unwrap();
    (PollFd::new(client).unwrap(), PollFd::new(server).unwrap())
}
