use std::{io, time::Duration};

use arrayvec::ArrayVec;
use compio::{
    driver::{AsRawFd, Driver, Entry, Operation, Poller},
    fs::File,
    op::ReadAt,
};

#[test]
fn cancel_before_poll() {
    let mut driver = Driver::new().unwrap();

    let file = File::open("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    driver.cancel(0);

    let mut op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(8));
    let ops = unsafe { [Operation::new_unchecked(&mut op, 0)] };
    let mut entries = ArrayVec::<Entry, 1>::new();

    let res = unsafe {
        driver.poll(
            Some(Duration::from_secs(1)),
            &mut ops.into_iter(),
            &mut entries,
        )
    };
    assert!(res.is_ok() || res.unwrap_err().kind() == io::ErrorKind::TimedOut);
}
