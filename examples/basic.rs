use compio::{driver::Poller, op::ReadAt};
use std::os::windows::prelude::{AsRawHandle, OpenOptionsExt};
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

fn main() {
    let runtime = compio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            .open("Cargo.toml")
            .unwrap();
        runtime.driver().attach(file.as_raw_handle()).unwrap();
        let op = ReadAt::new(file.as_raw_handle(), 0, Vec::with_capacity(1024));
        let (read, op) = runtime.submit(op).await;
        let read = read.unwrap();
        let mut buffer = op.into_buffer();
        unsafe {
            buffer.set_len(read);
        }
        println!("{}", std::str::from_utf8(&buffer).unwrap());
    })
}
