use std::{io, time::Duration};

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_driver::{
    op::{Asyncify, CloseFile, ReadAt},
    OpCode, Proactor, PushEntry, RawFd,
};

#[cfg(windows)]
fn open_file_op() -> impl OpCode {
    use std::os::windows::fs::OpenOptionsExt;

    use compio_driver::IntoRawFd;
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

    use compio_driver::op::OpenFile;

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
fn timeout() {
    let mut driver = Proactor::new().unwrap();

    let mut entries = ArrayVec::<usize, 1>::new();
    let err = driver
        .poll(Some(Duration::from_secs(1)), &mut entries)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
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
fn notify() {
    let mut driver = Proactor::new().unwrap();

    let handle = driver.handle().unwrap();

    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        handle.notify().unwrap()
    });

    let mut entries = ArrayVec::<usize, 1>::new();
    driver.poll(None, &mut entries).unwrap();

    thread.join().unwrap();
}

#[test]
fn asyncify() {
    let mut driver = Proactor::new().unwrap();

    let op = Asyncify::new(|| BufResult(Ok(114514), ()));
    let (res, _) = push_and_wait(&mut driver, op);
    assert_eq!(res, 114514);
}
