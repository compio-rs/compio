#![cfg(unix)]

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

use compio_driver::{AsFd, AsRawFd, AsSocketState, BorrowedFd, Proactor, PushEntry, op::Recv};
use rustix::net::RecvFlags;

struct Socket(UnixStream);

impl AsFd for Socket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsSocketState for Socket {}

/// Dropping the proactor must end every in-flight op: a receive left pending
/// in the kernel would consume data sent afterwards and write it into the
/// freed buffer.
///
/// A duplicate of the driver fd (as a `fork` without `exec` leaves) keeps an
/// io_uring ring alive after the proactor closes its own fd, so only an
/// explicit cancel ends the receive.
#[test]
fn drop_ends_pending_recv() {
    let (mut local, mut peer) = UnixStream::pair().unwrap();
    // The poll driver tries the receive inline: it must not block.
    local.set_nonblocking(true).unwrap();
    let mut driver = Proactor::new().unwrap();
    // SAFETY: the driver fd is open until `driver` is dropped.
    let _ring = rustix::io::dup(unsafe { BorrowedFd::borrow_raw(driver.as_raw_fd()) }).unwrap();

    let op = Recv::new(
        Socket(local.try_clone().unwrap()),
        Vec::with_capacity(64),
        RecvFlags::empty(),
    );
    let PushEntry::Pending(key) = driver.push(op) else {
        panic!("recv completed with no data sent");
    };
    // Submit the receive without waiting for it.
    match driver.poll(Some(Duration::ZERO)) {
        Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
        res => res.unwrap(),
    }
    drop(key);
    drop(driver);

    peer.write_all(b"x").unwrap();
    let mut buf = [0; 1];
    assert_eq!(
        local
            .read(&mut buf)
            .expect("data consumed by a receive that outlived the proactor"),
        1
    );
    assert_eq!(&buf, b"x");
}
