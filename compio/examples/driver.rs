use compio::{
    buf::{arrayvec::ArrayVec, IntoInner},
    driver::{op::ReadAt, AsRawFd, Entry, Proactor},
    fs::File,
};
use compio_driver::PushEntry;

fn main() {
    let mut driver = Proactor::new().unwrap();
    // Too hard to use OpenFile:(
    let file = compio::runtime::block_on(File::open("Cargo.toml")).unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(4096));
    let (n, op) = match driver.push(op) {
        PushEntry::Ready(res) => res.unwrap(),
        PushEntry::Pending(user_data) => {
            let mut entries = ArrayVec::<Entry, 1>::new();
            driver.poll(None, &mut entries).unwrap();
            let (n, op) = driver
                .pop(&mut entries.into_iter())
                .next()
                .unwrap()
                .unwrap();
            assert_eq!(op.user_data(), user_data);
            (n, unsafe { op.into_op() })
        }
    };

    let mut buffer = op.into_inner();
    unsafe {
        buffer.set_len(n);
    }
    println!("{}", String::from_utf8(buffer).unwrap());
}
