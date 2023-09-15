use std::{io, time::Duration};

use arrayvec::ArrayVec;
use compio::{
    driver::{AsRawFd, Driver, Entry, Poller},
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
    let ops = [(&mut op, 0).into()];
    let mut entries = ArrayVec::<Entry, 1>::new();

    let err = unsafe {
        driver
            .poll(
                Some(Duration::from_secs(1)),
                &mut ops.into_iter(),
                &mut entries,
            )
            .unwrap_err()
    };
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}
