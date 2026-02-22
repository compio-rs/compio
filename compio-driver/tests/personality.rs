#![cfg(io_uring)]

use compio_buf::BufResult;
use compio_driver::{
    op::{CurrentDir, ReadAt},
    *,
};

fn push_and_wait<O: OpCode + 'static>(
    driver: &mut Proactor,
    op: O,
    personality: u16,
) -> BufResult<usize, O> {
    let extra = driver.default_extra().with_personality(personality);
    assert!(extra.get_personality() == Some(personality));
    match driver.push_with_extra(op, extra) {
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

fn open_file(driver: &mut Proactor, personality: u16) -> OwnedFd {
    use std::{ffi::CString, os::fd::FromRawFd};

    use compio_driver::op::OpenFile;

    let op = OpenFile::new(
        CurrentDir,
        CString::new("Cargo.toml").unwrap(),
        libc::O_CLOEXEC | libc::O_RDONLY,
        0o666,
    );
    let (fd, _) = push_and_wait(driver, op, personality).unwrap();
    unsafe { OwnedFd::from_raw_fd(fd as _) }
}

#[test]
fn read_with_personality() {
    let mut driver = Proactor::new().expect("failed to create proactor");
    if !driver.driver_type().is_iouring() {
        eprintln!("Current driver does not support personality, skipping test");
        return;
    }
    let personality = driver
        .register_personality()
        .expect("failed to register personality");

    let fd = open_file(&mut driver, personality);
    let fd = SharedFd::new(fd);
    driver.attach(fd.as_raw_fd()).unwrap();

    push_and_wait(
        &mut driver,
        ReadAt::new(fd, 0, Vec::with_capacity(1024)),
        personality,
    )
    .expect("read failed");
}
