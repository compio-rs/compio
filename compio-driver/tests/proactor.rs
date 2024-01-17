use std::{io, time::Duration};

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_driver::{op::Asyncify, Proactor};

mod utils;

use utils::push_and_wait;

#[test]
fn timeout() {
    let mut driver = Proactor::new().unwrap();

    let mut entries = ArrayVec::<usize, 1>::new();
    let err = driver
        .poll(Some(Duration::from_secs(1)), &mut entries)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn notify() {
    let mut driver = Proactor::new().unwrap();

    let handle = driver.handle().unwrap();

    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        handle.notify().unwrap()
    });

    let mut entries = ArrayVec::<usize, 1>::new();
    driver.poll(None, &mut entries).unwrap();

    thread.join().unwrap();
}

#[test]
fn asyncify() {
    let mut driver = Proactor::new().unwrap();

    let op = Asyncify::new(|| BufResult(Ok(114514), ()));
    let (res, _) = push_and_wait(&mut driver, op);
    assert_eq!(res, 114514);
}
