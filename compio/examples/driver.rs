use compio::{
    buf::{arrayvec::ArrayVec, IntoInner},
    driver::{
        op::{OpenFile, ReadAt},
        AsRawFd, Entry, FromRawFd, OpCode, Proactor, PushEntry,
    },
    fs::File,
};

#[cfg(windows)]
fn open_file_op() -> OpenFile {
    use std::ptr::null_mut;

    use widestring::U16CString;
    use windows_sys::Win32::{
        Foundation::GENERIC_READ,
        Storage::FileSystem::{
            FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
    };

    OpenFile::new(
        U16CString::from_str("Cargo.toml").unwrap(),
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        null_mut(),
        OPEN_EXISTING,
        FILE_FLAG_OVERLAPPED,
    )
}

#[cfg(unix)]
fn open_file_op() -> OpenFile {
    use std::ffi::CString;

    let mut flags = libc::O_CLOEXEC | libc::O_RDONLY;
    if cfg!(not(any(target_os = "linux", target_os = "android"))) {
        flags |= libc::O_NONBLOCK;
    }

    OpenFile::new(CString::new("Cargo.toml").unwrap(), flags, 0o666)
}

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> (usize, O) {
    match driver.push(op) {
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
    }
}

fn main() {
    let mut driver = Proactor::new().unwrap();

    let op = open_file_op();
    let (fd, _) = push_and_wait(&mut driver, op);

    let file = unsafe { File::from_raw_fd(fd as _) };
    driver.attach(file.as_raw_fd()).unwrap();

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(4096));
    let (n, op) = push_and_wait(&mut driver, op);

    let mut buffer = op.into_inner();
    unsafe {
        buffer.set_len(n);
    }
    println!("{}", String::from_utf8(buffer).unwrap());
}
