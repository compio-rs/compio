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
    listener
        .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
        .unwrap();
    listener.listen(4).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = PollFd::new(listener).unwrap();
    let accept_task = listener.accept();

    let client = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    let client = PollFd::new(client).unwrap();
    let connect_task = client.connect(&addr);
    let ((mut tx, _), _) = futures_util::try_join!(accept_task, connect_task).unwrap();

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
async fn poll_connect_refused() {
    let listener = std::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = SockAddr::from(listener.local_addr().unwrap());
    drop(listener);

    let client = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    let client = PollFd::new(client).unwrap();
    let err = compio_runtime::time::timeout(Duration::from_secs(10), client.connect(&addr))
        .await
        .expect("connecting to a closed port timed out")
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
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

#[compio_macros::test]
async fn poll_concurrent_read_write_readiness() {
    let (peer, server) = connected_sockets();
    // Keep the buffers small so the server's send buffer can be filled
    // quickly. FD_WRITE must not become ready until the peer drains it.
    server.set_send_buffer_size(4 * 1024).unwrap();
    peer.set_recv_buffer_size(4 * 1024).unwrap();
    let queued = fill_send_buffer(&server);
    assert!(queued > 0, "expected the server send buffer to fill up");

    let poll_fd = PollFd::new(server).unwrap();

    let (read_started_tx, read_started_rx) = futures_channel::oneshot::channel();
    let (write_started_tx, write_started_rx) = futures_channel::oneshot::channel();
    let (read_done_tx, read_done_rx) = futures_channel::oneshot::channel();
    let (write_done_tx, write_done_rx) = futures_channel::oneshot::channel();

    let mut read_started_tx = Some(read_started_tx);
    let mut write_started_tx = Some(write_started_tx);

    let read_task = async {
        let res = std::future::poll_fn(|cx| {
            let res = poll_fd.poll_read_ready(cx);
            if let Some(tx) = read_started_tx.take() {
                assert!(
                    res.is_pending(),
                    "read readiness should be pending before the peer sends data"
                );
                let _ = tx.send(());
            }
            res
        })
        .await;
        let _ = read_done_tx.send(());
        res
    };

    let write_task = async {
        let res = std::future::poll_fn(|cx| {
            let res = poll_fd.poll_write_ready(cx);
            if let Some(tx) = write_started_tx.take() {
                assert!(
                    res.is_pending(),
                    "write readiness should be pending before the peer drains the send buffer"
                );
                let _ = tx.send(());
            }
            res
        })
        .await;
        let _ = write_done_tx.send(());
        res
    };

    let peer_task = async {
        read_started_rx.await.unwrap();
        write_started_rx.await.unwrap();

        // Trigger FD_READ first. The buggy Windows implementation lost the
        // still-pending FD_WRITE registration while clearing this event.
        peer.send(b"trigger").unwrap();
        read_done_rx.await.unwrap();

        // Drain the queued data so the server send buffer gains space and
        // FD_WRITE should fire.
        let mut buffer = Vec::with_capacity(1024);
        let mut drained = 0;
        loop {
            match peer.recv(buffer.spare_capacity_mut()) {
                Ok(0) => panic!("unexpected EOF while draining"),
                Ok(n) => {
                    drained += n;
                }
                Err(e) if is_would_block(&e) => break,
                Err(e) => panic!("failed to drain peer: {e}"),
            }
        }
        assert!(drained > 0, "peer received no queued data");

        write_done_rx.await.unwrap();
    };

    let (read, write, ()) = compio_runtime::time::timeout(Duration::from_secs(10), async {
        futures_util::join!(read_task, write_task, peer_task)
    })
    .await
    .expect("poll readiness tasks did not complete in time");
    read.unwrap();
    write.unwrap();
}

#[compio_macros::test]
async fn poll_close_half_closes() {
    let (mut tx, rx) = connected_pair();

    futures_util::AsyncWriteExt::write_all(&mut tx, b"Hello world!")
        .await
        .unwrap();
    futures_util::AsyncWriteExt::close(&mut tx).await.unwrap();
    // Closing twice is not an error: the second shutdown may report that the
    // write half is already gone.
    futures_util::AsyncWriteExt::close(&mut tx).await.unwrap();

    // The peer must see EOF while `tx` is still alive, so this cannot be the
    // FIN that the kernel sends when the descriptor is dropped.
    let mut buffer = Vec::with_capacity(32);
    let read_to_eof = async {
        let mut total = 0;
        loop {
            let read = std::future::poll_fn(|cx| {
                rx.poll_read_with(cx, |rx| {
                    // `spare_capacity_mut` already starts at `total`, since the
                    // length is updated on every iteration.
                    let n = rx.recv(buffer.spare_capacity_mut())?;
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
    // A file has no write half to shut down, so closing it must still succeed.
    let mut file = PollFd::new(tempfile::tempfile().unwrap()).unwrap();

    futures_util::AsyncWriteExt::write_all(&mut file, b"Hello world!")
        .await
        .unwrap();
    futures_util::AsyncWriteExt::close(&mut file).await.unwrap();
}

#[cfg(unix)]
#[compio_macros::test]
async fn poll_socket_operations_reject_files() {
    let file = PollFd::new(tempfile::tempfile().unwrap()).unwrap();
    let addr = SockAddr::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1));

    assert!(file.accept().await.is_err());
    assert!(file.connect(&addr).await.is_err());
}

fn connected_pair() -> (PollFd<Socket>, PollFd<Socket>) {
    let (client, server) = connected_sockets();
    (PollFd::new(client).unwrap(), PollFd::new(server).unwrap())
}

fn connected_sockets() -> (Socket, Socket) {
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
    (client, server)
}

fn fill_send_buffer(socket: &Socket) -> usize {
    let chunk = vec![0u8; 8 * 1024];
    let mut total = 0;
    loop {
        match socket.send(&chunk) {
            Ok(n) => total += n,
            Err(e) if is_would_block(&e) => return total,
            Err(e) => panic!("failed to fill send buffer: {e}"),
        }
    }
}
