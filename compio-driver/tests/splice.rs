#![cfg(linux_all)]

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsRawFd, OpCode, Proactor, PushEntry, SharedFd,
    op::{Read, Splice, Write},
};
use nix::{fcntl::OFlag, unistd::pipe2};

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> BufResult<usize, O> {
    match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut user_data) => loop {
            driver.poll(None).unwrap();
            match driver.pop(user_data) {
                PushEntry::Pending(k) => user_data = k,
                PushEntry::Ready(res) => break res,
            }
        },
    }
}

#[test]
fn splice() {
    let mut driver = Proactor::new().unwrap();

    let mut flags = OFlag::O_CLOEXEC;
    if driver.driver_type().is_polling() {
        flags |= OFlag::O_NONBLOCK;
    }

    let (rx, tx) = pipe2(flags).unwrap();
    let (rx1, tx1) = pipe2(flags).unwrap();
    println!(
        "rx={}, tx={}, rx1={}, tx1={}",
        rx.as_raw_fd(),
        tx.as_raw_fd(),
        rx1.as_raw_fd(),
        tx1.as_raw_fd()
    );

    let write_op = Write::new(SharedFd::new(tx), b"hello world");
    let read_op = Read::new(SharedFd::new(rx1), Vec::with_capacity(11));
    let splice_op = Splice::new(
        SharedFd::new(rx),
        -1,
        SharedFd::new(tx1),
        -1,
        11,
        libc::SPLICE_F_NONBLOCK,
    );

    push_and_wait(&mut driver, write_op).unwrap();
    push_and_wait(&mut driver, splice_op).unwrap();
    let (res, op) = push_and_wait(&mut driver, read_op).unwrap();
    let mut buf = op.into_inner();
    assert_eq!(res, 11);
    unsafe { buf.set_len(res) }
    assert_eq!(&buf[..], b"hello world");
}
