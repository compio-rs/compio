use std::{io, time::Duration};

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_driver::{
    op::{Asyncify, CloseFile, OpenFile, ReadAt},
    OpCode, Proactor, PushEntry, RawFd,
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

#[test]
#[cfg(all(target_os = "linux", not(feature = "polling")))]
fn push_128() {
    use compio_driver::OpEntry;

    struct ReadAt128(ReadAt<Vec<u8>>);

    impl OpCode for ReadAt128 {
        fn create_entry(self: std::pin::Pin<&mut Self>) -> compio_driver::OpEntry {
            if let OpEntry::Submission(entry) =
                unsafe { self.map_unchecked_mut(|op| &mut op.0) }.create_entry()
            {
                OpEntry::Submission128(entry.into())
            } else {
                unreachable!()
            }
        }
    }

    let mut driver = Proactor::new().unwrap();

    let op = open_file_op();
    let (fd, _) = push_and_wait(&mut driver, op);
    let fd = fd as RawFd;
    driver.attach(fd).unwrap();

    let op = ReadAt128(ReadAt::new(fd, 0, Vec::with_capacity(1024)));
    push_and_wait(&mut driver, op);

    let op = CloseFile::new(fd);
    push_and_wait(&mut driver, op);
}
