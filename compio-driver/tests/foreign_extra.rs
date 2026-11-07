//! An `Extra` created for one type of the fusion driver must be rejected by
//! the other: the io_uring driver used to panic after queueing the SQE, which
//! left the kernel holding a freed key.

#![cfg(all(target_os = "linux", feature = "io-uring", feature = "polling"))]

use std::{io, os::unix::net::UnixStream};

use compio_buf::BufResult;
use compio_driver::{
    DriverType, Extra, Proactor, PushEntry, SharedFd,
    op::{Interest, PollOnce},
};

fn proactor(driver_type: DriverType) -> Option<Proactor> {
    let driver = Proactor::builder().driver_type(driver_type).build().ok()?;
    (driver.driver_type() == driver_type).then_some(driver)
}

fn assert_rejected(driver: &mut Proactor, extra: Extra) {
    let (socket, _peer) = UnixStream::pair().unwrap();
    let op = PollOnce::new(SharedFd::new(socket), Interest::Readable);
    match driver.push_with_extra(op, extra) {
        PushEntry::Ready(BufResult(Err(e), _)) => assert_eq!(e.kind(), io::ErrorKind::InvalidInput),
        PushEntry::Ready(BufResult(Ok(_), _)) => panic!("op with a foreign `Extra` completed"),
        PushEntry::Pending(_) => panic!("op with a foreign `Extra` was submitted"),
    }
}

#[test]
fn push_with_foreign_extra_fails() {
    let (Some(mut iour), Some(mut poll)) =
        (proactor(DriverType::IoUring), proactor(DriverType::Poll))
    else {
        return;
    };
    assert_rejected(&mut iour, poll.default_extra());
    assert_rejected(&mut poll, iour.default_extra());
}
