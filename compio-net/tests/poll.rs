use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
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
        loop {
            listener.accept_ready().await.unwrap();
            match listener.accept() {
                Ok(res) => break res,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => panic!("{e:?}"),
            }
        }
    };

    let client = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    client.set_nonblocking(true).unwrap();
    let client = PollFd::new(client).unwrap();
    let res = client.connect(&addr);
    let tx = if let Err(e) = res {
        assert!(is_would_block(&e));
        let (tx, _) = accept_task.await;
        tx
    } else {
        let ((tx, _), res) = futures_util::join!(accept_task, client.connect_ready());
        res.unwrap();
        tx
    };

    tx.set_nonblocking(true).unwrap();
    let tx = PollFd::new(tx).unwrap();

    let send_task = async {
        loop {
            match tx.send(b"Hello world!") {
                Ok(res) => break res,
                Err(e) if is_would_block(&e) => {}
                Err(e) => panic!("{e:?}"),
            }
            tx.write_ready().await.unwrap();
        }
    };

    let mut buffer = Vec::with_capacity(12);
    let recv_task = async {
        loop {
            match client.recv(buffer.spare_capacity_mut()) {
                Ok(res) => {
                    unsafe { buffer.set_len(res) };
                    break res;
                }
                Err(e) if is_would_block(&e) => {}
                Err(e) => panic!("{e:?}"),
            }
            client.read_ready().await.unwrap();
        }
    };

    let (write, read) = futures_util::join!(send_task, recv_task);
    assert_eq!(write, 12);
    assert_eq!(read, 12);
    assert_eq!(buffer, b"Hello world!");
}
