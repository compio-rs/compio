//! Regression tests for operations on fds that cannot be monitored for
//! readiness, e.g. regular files on `epoll` and `/dev/null` on `kqueue`.
//!
//! The poll driver used to surface the registration error (`EPERM` on epoll,
//! `EINVAL` on kqueue) without ever performing the operation, so writing to
//! `stdout` redirected to `/dev/null` failed while `write(2)` succeeded. Such
//! operations are now executed on the blocking pool instead.

#![cfg(polling)]

use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    DriverType, Key, OpCode, Proactor, PushEntry, SharedFd,
    op::{Interest, PollOnce, Read, Write},
};

fn poll_driver() -> Proactor {
    Proactor::builder()
        .driver_type(DriverType::Poll)
        .build()
        .unwrap()
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "compio-poll-non-pollable-{}-{name}.tmp",
        std::process::id()
    ))
}

fn take_key<T: OpCode, R>(res: PushEntry<Key<T>, R>) -> Key<T> {
    match res {
        PushEntry::Pending(key) => key,
        PushEntry::Ready(_) => panic!("the operation completed immediately"),
    }
}

fn wait_for<T: OpCode>(driver: &mut Proactor, mut key: Key<T>) -> BufResult<usize, T> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "the operation stalled");
        match driver.poll(Some(Duration::from_millis(10))) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::ETIMEDOUT) => {}
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => panic!("poll failed: {e}"),
        }
        key = match driver.pop(key) {
            PushEntry::Pending(key) => key,
            PushEntry::Ready(res) => return res,
        };
    }
}

fn push_and_wait<T: OpCode + 'static>(driver: &mut Proactor, op: T) -> BufResult<usize, T> {
    let key = take_key(driver.push(op));
    wait_for(driver, key)
}

#[test]
fn write_regular_file() {
    let mut driver = poll_driver();
    let path = temp_path("write-regular-file");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    let fd = SharedFd::new(file);

    // Submit more than once: a failed registration must not leave the fd behind
    // in the poller, or the next attempt would fail with `AlreadyExists` on
    // kqueue.
    for i in 0..4u8 {
        let BufResult(res, _op) =
            push_and_wait(&mut driver, Write::new(fd.clone(), vec![b'a' + i; 1024]));
        assert_eq!(res.unwrap(), 1024);
    }
    drop(fd);

    assert_eq!(std::fs::metadata(&path).unwrap().len(), 4 * 1024);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn write_dev_null() {
    let mut driver = poll_driver();
    let fd = SharedFd::new(
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap(),
    );

    for _ in 0..3 {
        let BufResult(res, _op) =
            push_and_wait(&mut driver, Write::new(fd.clone(), vec![b'x'; 64]));
        assert_eq!(res.unwrap(), 64);
    }
}

#[test]
fn read_regular_file() {
    let mut driver = poll_driver();
    let path = temp_path("read-regular-file");
    std::fs::write(&path, b"hello world").unwrap();
    let fd = SharedFd::new(std::fs::File::open(&path).unwrap());

    let BufResult(res, op) = push_and_wait(&mut driver, Read::new(fd, vec![0u8; 16]));
    let n = res.unwrap();
    assert_eq!(n, 11);
    assert_eq!(&op.into_inner()[..n], b"hello world");

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn read_dev_null() {
    let mut driver = poll_driver();
    let fd = SharedFd::new(std::fs::File::open("/dev/null").unwrap());

    let BufResult(res, _op) = push_and_wait(&mut driver, Read::new(fd, vec![0u8; 8]));
    assert_eq!(res.unwrap(), 0);
}

#[test]
fn poll_once_regular_file() {
    let mut driver = poll_driver();
    let path = temp_path("poll-once-regular-file");
    std::fs::write(&path, b"hello").unwrap();
    let fd = SharedFd::new(std::fs::File::open(&path).unwrap());

    let key = take_key(driver.push(PollOnce::new(fd, Interest::Readable)));
    let BufResult(res, _op) = wait_for(&mut driver, key);
    assert_eq!(res.unwrap(), 0);

    std::fs::remove_file(&path).unwrap();
}

/// An operation whose fds are only partially monitorable must keep reporting
/// the registration error instead of falling back to the blocking pool: there
/// is a fd that could have woken the operation up, so the driver must not
/// execute it blindly.
#[cfg(target_os = "linux")]
#[test]
fn splice_partially_pollable_fds() {
    use compio_driver::op::{Splice, SpliceFlags};

    let mut driver = poll_driver();
    let path = temp_path("splice");
    std::fs::write(&path, b"hello world").unwrap();
    let file = SharedFd::new(std::fs::File::open(&path).unwrap());
    let (reader, writer) = io::pipe().unwrap();
    let op = Splice::new(
        file,
        -1,
        SharedFd::new(writer),
        -1,
        11,
        SpliceFlags::empty(),
    );

    match driver.push(op) {
        PushEntry::Ready(BufResult(res, _op)) => {
            // `epoll` refuses the regular file, while the pipe is monitorable.
            assert_eq!(res.unwrap_err().raw_os_error(), Some(libc::EPERM));
        }
        PushEntry::Pending(_) => panic!("`Splice` must not fall back to the blocking pool"),
    }

    drop(reader);
    std::fs::remove_file(&path).unwrap();
}
