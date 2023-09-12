use std::time::Duration;

use compio::driver::{Driver, Poller};

#[test]
fn poll_zero() {
    let mut driver = Driver::new().unwrap();
    let polled = driver.poll(Some(Duration::from_nanos(0))).unwrap();
    assert_eq!(polled, 0);
}
