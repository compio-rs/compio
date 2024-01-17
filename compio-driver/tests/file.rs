use std::io;

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_driver::{
    op::{CloseFile, OpenFile, ReadAt},
    Proactor, PushEntry, RawFd,
};

mod utils;

use utils::push_and_wait;

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

#[test]
fn cancel_before_poll() {
    let mut driver = Proactor::new().unwrap();

    let op = open_file_op();
    let (fd, _) = push_and_wait(&mut driver, op);
    let fd = fd as RawFd;
    driver.attach(fd).unwrap();

    driver.cancel(0);

    let op = ReadAt::new(fd, 0, Vec::with_capacity(8));
    let BufResult(res, _) = match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(key) => {
            let mut entries = ArrayVec::<usize, 1>::new();
            driver.poll(None, &mut entries).unwrap();
            assert_eq!(entries[0], *key);
            driver.pop(key)
        }
    };

    assert!(res.is_ok() || res.unwrap_err().kind() == io::ErrorKind::TimedOut);

    let op = CloseFile::new(fd);
    push_and_wait(&mut driver, op);
}

#[test]
fn register_multiple() {
    const TASK_LEN: usize = 5;

    let mut driver = Proactor::new().unwrap();

    let op = open_file_op();
    let (fd, _) = push_and_wait(&mut driver, op);
    let fd = fd as RawFd;
    driver.attach(fd).unwrap();

    let mut need_wait = 0;

    for _i in 0..TASK_LEN {
        match driver.push(ReadAt::new(fd, 0, Vec::with_capacity(1024))) {
            PushEntry::Pending(_) => need_wait += 1,
            PushEntry::Ready(res) => {
                res.unwrap();
            }
        }
    }

    let mut entries = ArrayVec::<usize, TASK_LEN>::new();
    while entries.len() < need_wait {
        driver.poll(None, &mut entries).unwrap();
    }

    let op = CloseFile::new(fd);
    push_and_wait(&mut driver, op);
}

#[test]
#[cfg(all(target_os = "linux", feature = "io-uring"))]
fn custom_op() {
    use compio_driver::op::IoUringOp;

    let mut driver = Proactor::new().unwrap();

    let op = open_file_op();
    let (fd, _) = push_and_wait(&mut driver, op);
    let fd = fd as RawFd;
    driver.attach(fd).unwrap();

    let mut buffer = vec![0u8; 1024];

    let op = IoUringOp::new(
        io_uring::opcode::Read::new(
            io_uring::types::Fd(fd),
            buffer.as_mut_ptr(),
            buffer.len() as _,
        )
        .build(),
    );
    let (len, _) = push_and_wait(&mut driver, op);
    assert!(len > 0);

    let op = CloseFile::new(fd);
    push_and_wait(&mut driver, op);
}
