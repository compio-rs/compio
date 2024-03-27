use compio::{
    buf::{arrayvec::ArrayVec, IntoInner},
    driver::{
        op::{CloseFile, ReadAt},
        OpCode, Proactor, PushEntry, RawFd,
    },
};

#[cfg(windows)]
fn open_file_op() -> impl OpCode {
    use std::os::windows::fs::OpenOptionsExt;

    use compio::{
        driver::{op::Asyncify, IntoRawFd},
        BufResult,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

    Asyncify::new(|| {
        BufResult(
            std::fs::OpenOptions::new()
                .read(true)
                .attributes(FILE_FLAG_OVERLAPPED)
                .open("Cargo.toml")
                .map(|f| f.into_raw_fd() as usize),
            (),
        )
    })
}

#[cfg(unix)]
fn open_file_op() -> impl OpCode {
    use std::ffi::CString;

    use compio::driver::op::OpenFile;

    OpenFile::new(
        CString::new("Cargo.toml").unwrap(),
        libc::O_CLOEXEC | libc::O_RDONLY,
        0o666,
    )
}

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> (usize, O) {
    match driver.push(op) {
        PushEntry::Ready(res) => res.unwrap(),
        PushEntry::Pending(user_data) => {
            let mut entries = ArrayVec::<usize, 1>::new();
            while entries.is_empty() {
                driver.poll(None, &mut entries).unwrap();
            }
            assert_eq!(entries[0], *user_data);
            driver.pop(user_data).unwrap()
        }
    }
}

fn main() {
    let mut driver = Proactor::new().unwrap();

    let op = open_file_op();
    let (fd, _) = push_and_wait(&mut driver, op);
    let fd = fd as RawFd;

    driver.attach(fd).unwrap();

    let op = ReadAt::new(fd, 0, Vec::with_capacity(4096));
    let (n, op) = push_and_wait(&mut driver, op);

    let mut buffer = op.into_inner();
    unsafe {
        buffer.set_len(n);
    }
    println!("{}", String::from_utf8(buffer).unwrap());

    let op = CloseFile::new(fd);
    push_and_wait(&mut driver, op);
}
