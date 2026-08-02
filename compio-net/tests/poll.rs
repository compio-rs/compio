use std::{
    io::{self, IoSlice},
    net::{Ipv4Addr, SocketAddrV4},
    pin::Pin,
    time::Duration,
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
    assert_eq!(
        written, 12,
        "expected a single writev covering every buffer"
    );

    let mut buffer = Vec::with_capacity(12);
    let read = std::future::poll_fn(|cx| {
        rx.poll_read_with(cx, |rx| {
            let n = rx.recv(buffer.spare_capacity_mut())?;
            unsafe { buffer.set_len(n) };
            Ok(n)
        })
    })
    .await
    .unwrap();

    assert_eq!(read, 12);
    assert_eq!(buffer, b"Hello world!");
}

#[compio_macros::test]
async fn poll_close_half_closes() {
    let (mut tx, rx) = connected_pair();

    futures_util::AsyncWriteExt::write_all(&mut tx, b"Hello world!")
        .await
        .unwrap();
    futures_util::AsyncWriteExt::close(&mut tx).await.unwrap();

    // The peer must see EOF while `tx` is still alive, so this cannot be the
    // FIN that the kernel sends when the descriptor is dropped.
    let mut buffer = Vec::with_capacity(32);
    let read_to_eof = async {
        let mut total = 0;
        loop {
            let read = std::future::poll_fn(|cx| {
                rx.poll_read_with(cx, |rx| {
                    let n = rx.recv(&mut buffer.spare_capacity_mut()[total..])?;
                    unsafe { buffer.set_len(total + n) };
                    Ok(n)
                })
            })
            .await
            .unwrap();
            if read == 0 {
                break;
            }
            total += read;
        }
    };
    compio_runtime::time::timeout(Duration::from_secs(10), read_to_eof)
        .await
        .expect("closing the write half did not make the peer observe EOF");

    assert_eq!(buffer, b"Hello world!");
    drop(tx);
}

#[cfg(unix)]
#[compio_macros::test]
async fn poll_close_ignores_sources_without_a_write_half() {
    use std::os::fd::{FromRawFd, OwnedFd};

    // A pipe has no write half to shut down, so closing it must still succeed.
    let mut fds = [0; 2];
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "{}", io::Error::last_os_error());
    let [reader, writer] = fds;
    let reader = unsafe { OwnedFd::from_raw_fd(reader) };
    let writer = unsafe { OwnedFd::from_raw_fd(writer) };

    PollFd::new(writer).unwrap().shutdown_write().unwrap();

    drop(reader);
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
