use std::{
    io::{self, Write as _},
    net::{TcpListener, TcpStream},
    time::Duration,
};

use compio_buf::BufResult;
#[cfg(unix)]
use compio_driver::op::{AcceptMulti, Pipe, ReadMulti, Write};
use compio_driver::{
    AsRawFd, Extra, OpCode, OwnedFd, Proactor, PushEntry, SharedFd, TakeBuffer,
    op::{Asyncify, CloseFile, CloseSocket, ReadAt, ReadManagedAt, RecvMulti, ResultTakeBuffer},
};

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

fn push_and_wait_multi<O: OpCode + 'static>(
    driver: &mut Proactor,
    op: O,
) -> impl Iterator<Item = BufResult<usize, (Extra, Option<O>)>> + '_ {
    let mut op = Some(op);
    let mut user_data = None;
    let mut finished = false;

    std::iter::from_fn(move || {
        if finished {
            return None;
        }

        if user_data.is_none() {
            match driver.push(op.take().expect("operation should be pushed once")) {
                PushEntry::Ready(BufResult(res, op)) => {
                    finished = true;
                    return Some(BufResult(res, (driver.default_extra(), Some(op))));
                }
                PushEntry::Pending(k) => user_data = Some(k),
            }
        }

        loop {
            if let Some(res) = user_data.as_ref().and_then(|key| driver.pop_multishot(key)) {
                return Some(res.map_buffer(|extra| (extra, None)));
            }

            let key = user_data.take().expect("pending key should exist");
            match driver.pop_with_extra(key) {
                PushEntry::Pending(k) => user_data = Some(k),
                PushEntry::Ready((BufResult(res, op), extra)) => {
                    finished = true;
                    return Some(BufResult(res, (extra, Some(op))));
                }
            }

            driver.poll(None).unwrap();
        }
    })
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
    use compio_buf::IntoInner;

    let mut driver = Proactor::new().unwrap();

    let op = Pipe::new();
    let (_, op) = push_and_wait(&mut driver, op).unwrap();
    let (r, _w) = op.into_inner();

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
#[cfg(unix)]
fn read_multi() {
    use compio_buf::IntoInner;

    let mut driver = Proactor::new().unwrap();

    let op = Pipe::new();
    let (_, op) = push_and_wait(&mut driver, op).unwrap();
    let (r, w) = op.into_inner();

    let op = Write::new(w, b"hello world");
    push_and_wait(&mut driver, op).unwrap();

    let pool = driver.create_buffer_pool(4, 1024).unwrap();

    let op = ReadMulti::new(r, &pool, 0).unwrap();
    let buffer = push_and_wait_multi(&mut driver, op)
        .map(|BufResult(res, (extra, op))| {
            if let Some(op) = op {
                (BufResult(res, op), extra).take_buffer(&pool)
            } else {
                BufResult(res, extra).take_buffer(&pool)
            }
            .map(|buf| buf.to_vec())
            .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    assert_eq!(buffer, b"hello world");
}

#[test]
fn recv_multi() {
    let mut driver = Proactor::new().unwrap();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream.write_all(b"hello ").unwrap();
        stream.write_all(b"world").unwrap();
        stream.shutdown(std::net::Shutdown::Both).unwrap();
    });

    let stream = TcpStream::connect(addr).unwrap();
    let stream = socket2::Socket::from(stream);
    if driver.driver_type().is_polling() {
        stream.set_nonblocking(true).unwrap();
    }
    let stream = SharedFd::new(stream);

    driver.attach(stream.as_raw_fd()).unwrap();

    let pool = driver.create_buffer_pool(4, 1024).unwrap();

    let mut buffer = vec![];
    loop {
        let op = RecvMulti::new(stream.clone(), &pool, 0, 0).unwrap();
        let slice = push_and_wait_multi(&mut driver, op)
            .map(|BufResult(res, (extra, op))| {
                if let Some(op) = op {
                    (BufResult(res, op), extra).take_buffer(&pool)
                } else {
                    BufResult(res, extra).take_buffer(&pool)
                }
                .map(|buf| buf.to_vec())
                .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if slice.is_empty() {
            break;
        }
        buffer.extend_from_slice(&slice);
    }
    assert_eq!(buffer, b"hello world");

    let stream = stream.try_unwrap().unwrap();
    let op = CloseSocket::new(stream.into());
    push_and_wait(&mut driver, op).unwrap();
}

#[cfg(unix)]
#[test]
fn accept_multi() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();

    let handle = std::thread::spawn(move || {
        let mut driver = Proactor::new().unwrap();

        let server = socket2::Socket::from(server);
        if driver.driver_type().is_polling() {
            server.set_nonblocking(true).unwrap();
        }
        let server = SharedFd::new(server);

        driver.attach(server.as_raw_fd()).unwrap();

        let mut i = 0;
        loop {
            let op = AcceptMulti::new(server.clone());
            for BufResult(res, (_, op)) in push_and_wait_multi(&mut driver, op) {
                let mut client = if let Some(op) = op {
                    use compio_buf::IntoInner;

                    op.into_inner()
                } else {
                    unsafe {
                        use std::os::fd::FromRawFd;
                        socket2::Socket::from_raw_fd(res.unwrap() as _)
                    }
                };
                client
                    .write_all(format!("Hello, {}", i).as_bytes())
                    .unwrap();
                client.shutdown(std::net::Shutdown::Both).unwrap();
                i += 1;
                if i >= 2 {
                    return;
                }
            }
        }
    });
    for i in 0..2 {
        use std::io::Read;

        let mut client = TcpStream::connect(addr).unwrap();
        let mut s = String::new();
        client.read_to_string(&mut s).unwrap();
        assert_eq!(s, format!("Hello, {}", i));
    }
    if let Err(e) = handle.join() {
        std::panic::resume_unwind(e)
    }
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

/// Dropping a Proactor with in-flight ops must not leak. Validated by ASan.
#[test]
fn drop_with_inflight_ops() {
    let mut driver = Proactor::builder().build().unwrap();

    let op = Asyncify::new(|| BufResult(Ok(0), ()));
    match driver.push(op) {
        PushEntry::Ready(_) => {}
        PushEntry::Pending(_) => {}
    }

    drop(driver);
}
