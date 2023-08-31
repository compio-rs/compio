use compio::buf::IntoInner;
use compio::driver::{AsRawFd, Driver, Poller};
use std::task::Poll;

fn main() {
    let driver = Driver::new().unwrap();
    let file = compio::fs::File::open("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    let mut op = compio::op::ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(4096));
    let res = match unsafe { driver.push(&mut op, 0) } {
        Poll::Ready(res) => res,
        Poll::Pending => {
            let entry = driver.poll(None).unwrap();
            assert_eq!(entry.user_data(), 0);
            entry.into_result()
        }
    };
    let n = res.unwrap();
    let mut buffer = op.into_inner().into_inner();
    unsafe {
        buffer.set_len(n);
    }
    println!("{}", String::from_utf8(buffer).unwrap());
}
