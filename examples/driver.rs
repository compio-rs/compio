use compio::{
    buf::IntoInner,
    driver::{AsRawFd, Driver, Poller},
};

fn main() {
    let driver = Driver::new().unwrap();
    let file = compio::fs::File::open("Cargo.toml").unwrap();
    let registered_fd = driver.attach(file.as_raw_fd()).unwrap();

    let mut op = compio::op::ReadAt::new(registered_fd, 0, Vec::with_capacity(4096));
    unsafe { driver.push(&mut op, 0) }.unwrap();

    let entry = driver.poll_one(None).unwrap();
    assert_eq!(entry.user_data(), 0);

    let n = entry.into_result().unwrap();
    let mut buffer = op.into_inner().into_inner();
    unsafe {
        buffer.set_len(n);
    }
    println!("{}", String::from_utf8(buffer).unwrap());
}
