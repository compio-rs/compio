use std::{io, time::Duration};

use arrayvec::ArrayVec;
use compio::{
    driver::{AsRawFd, Entry, Proactor},
    fs::File,
    op::ReadAt,
};

#[test]
fn cancel_before_poll() {
    let mut driver = Proactor::new().unwrap();

    let file = File::open("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    driver.cancel(0);

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(8));
    driver.push(op);

    let mut entries = ArrayVec::<Entry, 1>::new();
    let res = driver.poll(Some(Duration::from_secs(1)), &mut entries);
    assert!(res.is_ok() || res.unwrap_err().kind() == io::ErrorKind::TimedOut);
}
