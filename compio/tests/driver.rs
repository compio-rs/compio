use std::{io, path::Path, time::Duration};

use compio::{
    buf::{arrayvec::ArrayVec, BufResult},
    driver::{op::ReadAt, AsRawFd, Entry, Proactor},
    fs::File,
};
use compio_driver::PushEntry;

fn open_file(path: impl AsRef<Path>) -> io::Result<File> {
    compio::runtime::block_on(File::open(path))
}

#[test]
fn cancel_before_poll() {
    let mut driver = Proactor::new().unwrap();

    let file = open_file("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    driver.cancel(0);

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(8));
    let BufResult(res, _) = match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(key) => {
            let mut entries = ArrayVec::<Entry, 1>::new();
            driver.poll(None, &mut entries).unwrap();
            driver
                .pop(&mut entries.into_iter())
                .next()
                .unwrap()
                .map_buffer(|op| {
                    assert_eq!(op.user_data(), key);
                    unsafe { op.into_op() }
                })
        }
    };

    assert!(res.is_ok() || res.unwrap_err().kind() == io::ErrorKind::TimedOut);
}

#[test]
fn timeout() {
    let mut driver = Proactor::new().unwrap();

    let mut entries = ArrayVec::<Entry, 1>::new();
    let err = driver
        .poll(Some(Duration::from_secs(1)), &mut entries)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn register_multiple() {
    const TASK_LEN: usize = 5;

    let mut driver = Proactor::new().unwrap();

    let file = open_file("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    let mut need_wait = 0;

    for _i in 0..TASK_LEN {
        match driver.push(ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(1024))) {
            PushEntry::Pending(_) => need_wait += 1,
            PushEntry::Ready(res) => {
                res.unwrap();
            }
        }
    }

    let mut entries = ArrayVec::<Entry, TASK_LEN>::new();
    while entries.len() < need_wait {
        driver.poll(None, &mut entries).unwrap();
    }
}
