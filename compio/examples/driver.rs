use compio::{
    buf::{arrayvec::ArrayVec, IntoInner},
    driver::{AsRawFd, Entry, Proactor},
    op::ReadAt,
};

fn main() {
    let mut driver = Proactor::new().unwrap();
    let file = compio::fs::File::open("Cargo.toml").unwrap();
    #[cfg(not(feature = "runtime"))]
    driver.attach(file.as_raw_fd()).unwrap();

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(4096));
    let user_data = driver.push(op);

    let mut entries = ArrayVec::<Entry, 1>::new();
    driver.poll(None, &mut entries).unwrap();
    let (res, op) = driver.pop(&mut entries.into_iter()).next().unwrap();
    let n = res.unwrap();
    assert_eq!(op.user_data(), user_data);

    let mut buffer = unsafe { op.into_op::<ReadAt<Vec<u8>>>() }.into_inner();
    unsafe {
        buffer.set_len(n);
    }
    println!("{}", String::from_utf8(buffer).unwrap());
}
