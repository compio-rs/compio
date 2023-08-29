use compio::{
    driver::{AsRawFd, Poller},
    fs::OpenOptions,
    op::ReadAt,
};

fn main() {
    let runtime = compio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let file = OpenOptions::new().read(true).open("Cargo.toml").unwrap();
        runtime.driver().attach(file.as_raw_fd()).unwrap();
        let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(1024));
        let (read, op) = runtime.submit(op).await;
        let read = read.unwrap();
        let mut buffer = op.into_buffer();
        unsafe {
            buffer.set_len(read);
        }
        println!("{}", std::str::from_utf8(&buffer).unwrap());
    })
}
