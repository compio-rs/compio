use compio::driver::{Driver, Poller};

#[test]
fn poll_zero() {
    let driver = Driver::new().unwrap();
    let polled = driver.poll(None, &mut []).unwrap();
    assert_eq!(polled, 0);
}
