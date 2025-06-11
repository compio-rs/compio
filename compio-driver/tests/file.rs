use std::{io, time::Duration};

use compio_buf::BufResult;
use compio_driver::{
    AsRawFd, OpCode, OwnedFd, Proactor, PushEntry, SharedFd, TakeBuffer,
    op::{Asyncify, CloseFile, ReadAt, ReadManagedAt},
};

#[cfg(windows)]
fn open_file(driver: &mut Proactor) -> OwnedFd {
    use std::os::windows::{
        fs::OpenOptionsExt,
        io::{FromRawHandle, IntoRawHandle, OwnedHandle},
    };

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

    let op = Asyncify::new(|| {
        BufResult(
            std::fs::OpenOptions::new()
                .read(true)
                .attributes(FILE_FLAG_OVERLAPPED)
                .open("Cargo.toml")
                .map(|f| f.into_raw_handle() as usize),
            (),
        )
    });
    let (fd, _) = push_and_wait(driver, op).unwrap();
    OwnedFd::File(unsafe { OwnedHandle::from_raw_handle(fd as _) })
}

#[cfg(unix)]
fn open_file(driver: &mut Proactor) -> OwnedFd {
    use std::{ffi::CString, os::fd::FromRawFd};

    use compio_driver::op::OpenFile;

    let op = OpenFile::new(
        CString::new("Cargo.toml").unwrap(),
        libc::O_CLOEXEC | libc::O_RDONLY,
        0o666,
    );
    let (fd, _) = push_and_wait(driver, op).unwrap();
    unsafe { OwnedFd::from_raw_fd(fd as _) }
}

fn push_and_wait_flags<O: OpCode + 'static>(
    driver: &mut Proactor,
    op: O,
) -> (BufResult<usize, O>, u32) {
    match driver.push(op) {
        PushEntry::Ready(res) => (res, 0),
        PushEntry::Pending(mut user_data) => loop {
            driver.poll(None).unwrap();
            match driver.pop(user_data) {
                PushEntry::Pending(k) => user_data = k,
                PushEntry::Ready((res, flags)) => break (res, flags),
            }
        },
    }
}

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> BufResult<usize, O> {
    push_and_wait_flags(driver, op).0
}

#[test]
fn timeout() {
    let mut driver = Proactor::new().unwrap();

    let err = driver.poll(Some(Duration::from_secs(1))).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn register_multiple() {
    const TASK_LEN: usize = 5;

    let mut driver = Proactor::new().unwrap();

    let fd = open_file(&mut driver);
    let fd = SharedFd::new(fd);
    driver.attach(fd.as_raw_fd()).unwrap();

    let mut keys = vec![];

    for _i in 0..TASK_LEN {
        match driver.push(ReadAt::new(fd.clone(), 0, Vec::with_capacity(1024))) {
            PushEntry::Pending(key) => keys.push(key),
            PushEntry::Ready(res) => {
                res.unwrap();
            }
        }
    }

    driver.poll(None).unwrap();

    // Cancel the entries to drop the ops, and decrease the ref count of fd.
    for key in keys {
        driver.cancel(key);
    }

    // Don't async close because the reading operations may have not completed.
}

#[test]
fn notify() {
    let mut driver = Proactor::new().unwrap();

    let handle = driver.handle();

    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        handle.notify().unwrap()
    });

    driver.poll(None).unwrap();

    thread.join().unwrap();
}

#[test]
fn asyncify() {
    let mut driver = Proactor::new().unwrap();

    let op = Asyncify::new(|| BufResult(Ok(114514), ()));
    let (res, _) = push_and_wait(&mut driver, op).unwrap();
    assert_eq!(res, 114514);
}

#[test]
fn managed() {
    let mut driver = Proactor::new().unwrap();

    let fd = open_file(&mut driver);
    let fd = SharedFd::new(fd);
    driver.attach(fd.as_raw_fd()).unwrap();

    let pool = driver.create_buffer_pool(4, 1024).unwrap();

    let op = ReadManagedAt::new(fd.clone(), 0, &pool, 1024).unwrap();
    let (BufResult(res, op), flags) = push_and_wait_flags(&mut driver, op);

    let buffer = op.take_buffer(&pool, res, flags).unwrap();
    println!("{}", std::str::from_utf8(&buffer).unwrap());

    let op = CloseFile::new(fd.try_unwrap().unwrap());
    push_and_wait(&mut driver, op).unwrap();
}
