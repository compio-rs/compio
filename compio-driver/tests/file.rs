use std::{io, ops::Deref, time::Duration};

use compio_buf::BufResult;
use compio_driver::{
    AsRawFd, BufferPool, Extra, OpCode, OwnedFd, Proactor, PushEntry, SharedFd, TakeBuffer,
    op::{Asyncify, CloseFile, ReadAt, ReadManagedAt, ReadMulti, ResultTakeBuffer},
};
mod pipe2;

#[cfg(unix)]
#[test]
fn truncate_file_poll() {
    use compio_driver::{ToSharedFd, op::TruncateFile};

    let mut driver = Proactor::builder().build().unwrap();

    let fd = std::fs::File::create_new("temp.txt").unwrap();
    let file = SharedFd::new(fd);
    driver.attach(file.as_raw_fd()).unwrap();

    let size = 5;
    let op = TruncateFile::new(file.to_shared_fd(), size);
    let _ = push_and_wait(&mut driver, op);

    let meta = file.metadata().unwrap();
    std::fs::remove_file("temp.txt").unwrap();
    assert_eq!(5, meta.len());
}

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
    use std::ffi::CString;

    use compio_buf::IntoInner;
    use compio_driver::op::{CurrentDir, OpenFile};

    let op = OpenFile::new(
        CurrentDir,
        CString::new("Cargo.toml").unwrap(),
        libc::O_CLOEXEC | libc::O_RDONLY,
        0o666,
    );
    let (_, op) = push_and_wait(driver, op).unwrap();
    op.into_inner()
}

fn push_and_wait_extra<O: OpCode + 'static>(
    driver: &mut Proactor,
    op: O,
) -> (BufResult<usize, O>, Option<Extra>) {
    match driver.push(op) {
        PushEntry::Ready(res) => (res, None),
        PushEntry::Pending(mut user_data) => loop {
            driver.poll(None).unwrap();
            match driver.pop_with_extra(user_data) {
                PushEntry::Pending(k) => user_data = k,
                PushEntry::Ready((res, extra)) => break (res, Some(extra)),
            }
        },
    }
}

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> BufResult<usize, O> {
    match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut user_data) => loop {
            driver.poll(None).unwrap();
            match driver.pop(user_data) {
                PushEntry::Pending(k) => user_data = k,
                PushEntry::Ready(res) => break res,
            }
        },
    }
}

fn push_and_wait_multi<O: OpCode + TakeBuffer<BufferPool = BufferPool> + 'static>(
    driver: &mut Proactor,
    op: O,
    pool: &BufferPool,
) -> Vec<u8>
where
    for<'a> O::Buffer<'a>: Deref<Target = [u8]>,
{
    match driver.push(op) {
        PushEntry::Ready(res) => {
            let buf = (res, driver.default_extra()).take_buffer(pool).unwrap();
            buf.to_vec()
        }
        PushEntry::Pending(mut user_data) => {
            let mut buffer = vec![];
            loop {
                driver.poll(None).unwrap();
                if let Some(res) = driver.pop_multishot(&user_data) {
                    let slice = res.take_buffer(pool).unwrap();
                    buffer.extend_from_slice(&slice);
                } else {
                    match driver.pop_with_extra(user_data) {
                        PushEntry::Pending(k) => user_data = k,
                        PushEntry::Ready(res) => {
                            let slice = res.take_buffer(pool).unwrap();
                            buffer.extend_from_slice(&slice);
                            break;
                        }
                    }
                }
            }
            buffer
        }
    }
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
#[cfg(unix)]
fn cancel_token() {
    use nix::fcntl::OFlag;

    let mut driver = Proactor::new().unwrap();

    let mut flags = OFlag::O_CLOEXEC;
    if driver.driver_type().is_polling() {
        flags |= OFlag::O_NONBLOCK;
    }

    let (r, _w) = pipe2::pipe2(flags).unwrap();

    let mut key = match driver.push(ReadAt::new(r, 0, Vec::with_capacity(1024))) {
        PushEntry::Pending(key) => key,
        PushEntry::Ready(res) => {
            res.unwrap();
            return;
        }
    };

    let token = driver.register_cancel(&key);
    assert!(driver.cancel_token(token));

    let res = loop {
        driver.poll(None).unwrap();
        match driver.pop(key) {
            PushEntry::Pending(k) => key = k,
            PushEntry::Ready(res) => break res,
        }
    };

    assert!(res.0.is_err())
}

#[test]
fn notify() {
    let mut driver = Proactor::new().unwrap();

    let waker = driver.waker();

    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        waker.wake();
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
    let (BufResult(res, op), extra) = push_and_wait_extra(&mut driver, op);

    let buffer_id = extra.unwrap().buffer_id().expect("Buffer ID missing");

    let buffer = op.take_buffer(&pool, res, buffer_id).unwrap();
    println!("{}", std::str::from_utf8(&buffer).unwrap());

    let op = CloseFile::new(fd.try_unwrap().unwrap());
    push_and_wait(&mut driver, op).unwrap();
}

#[test]
fn read_multi() {
    let mut driver = Proactor::new().unwrap();

    let fd = open_file(&mut driver);
    let fd = SharedFd::new(fd);
    driver.attach(fd.as_raw_fd()).unwrap();

    let pool = driver.create_buffer_pool(4, 1024).unwrap();

    let op = ReadMulti::new(fd.clone(), &pool, 1024).unwrap();
    let buffer = push_and_wait_multi(&mut driver, op, &pool);

    println!("{}", std::str::from_utf8(&buffer).unwrap());

    let op = CloseFile::new(fd.try_unwrap().unwrap());
    push_and_wait(&mut driver, op).unwrap();
}

#[test]
#[cfg(all(target_pointer_width = "64", any(io_uring, target_os = "windows")))]
fn read_len_over_u32() {
    let mut driver = Proactor::new().unwrap();

    let fd = open_file(&mut driver);
    let fd = SharedFd::new(fd);
    driver.attach(fd.as_raw_fd()).unwrap();

    let buffer = Vec::with_capacity(1 << 32);

    let op = ReadAt::new(fd.clone(), 0, buffer);
    let (res, _) = push_and_wait(&mut driver, op).unwrap();

    assert!(res > 0);

    let op = CloseFile::new(fd.try_unwrap().unwrap());
    push_and_wait(&mut driver, op).unwrap();
}
