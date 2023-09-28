use std::{io, time::Duration};

use compio::{
    buf::arrayvec::ArrayVec,
    driver::{AsRawFd, Entry, Proactor},
    fs::File,
    op::ReadAt,
};

#[test]
fn cancel_before_poll() {
    let mut driver = Proactor::new().unwrap();

    let file = File::open("Cargo.toml").unwrap();
    #[cfg(not(feature = "runtime"))]
    driver.attach(file.as_raw_fd()).unwrap();

    driver.cancel(0);

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(8));
    let key = driver.push(op);

    let mut entries = ArrayVec::<Entry, 1>::new();
    driver.poll(None, &mut entries).unwrap();
    let (res, op) = driver.pop(&mut entries.into_iter()).next().unwrap();
    assert_eq!(op.user_data(), key);

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

    let file = File::open("Cargo.toml").unwrap();
    #[cfg(not(feature = "runtime"))]
    driver.attach(file.as_raw_fd()).unwrap();

    for _i in 0..TASK_LEN {
        driver.push(ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(1024)));
    }

    let mut entries = ArrayVec::<Entry, TASK_LEN>::new();
    while entries.len() < TASK_LEN {
        driver.poll(Some(Duration::from_secs(1)), &mut entries).unwrap();
        for entry in entries.iter_mut() {
            let _ = entry.take_result().unwrap();
        }
    }
}
