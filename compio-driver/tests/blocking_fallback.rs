//! Ops that create an fd must hand back an open fd when the driver runs them
//! on the blocking thread pool (e.g. io_uring opcode unsupported by the
//! running kernel). Runs on every driver available at runtime.

#![cfg(any(io_uring, polling))]

use std::{
    ffi::CString,
    net::{TcpListener, TcpStream},
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        unix::fs::MetadataExt,
    },
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsSocketState, DriverType, OpCode, Proactor, PushEntry,
    op::{Accept, AcceptMulti, CreateSocket, CurrentDir, Mode, OFlags, OpenFile},
};

/// Forces the blocking path, as the driver does for ops it can't submit.
struct ForceBlocking<T>(T);

#[cfg(io_uring)]
unsafe impl<T: compio_driver::IourOpCode> compio_driver::IourOpCode for ForceBlocking<T> {
    type Control = T::Control;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.0.init(ctrl) }
    }

    fn create_entry(&mut self, _: &mut Self::Control) -> compio_driver::OpEntry {
        compio_driver::OpEntry::Blocking
    }

    fn call_blocking(&mut self, ctrl: &mut Self::Control) -> std::io::Result<usize> {
        self.0.call_blocking(ctrl)
    }

    unsafe fn set_result(
        &mut self,
        ctrl: &mut Self::Control,
        res: &std::io::Result<usize>,
        extra: &compio_driver::Extra,
    ) {
        unsafe { self.0.set_result(ctrl, res, extra) }
    }
}

#[cfg(polling)]
unsafe impl<T: compio_driver::PollOpCode> compio_driver::PollOpCode for ForceBlocking<T> {
    type Control = T::Control;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.0.init(ctrl) }
    }

    fn pre_submit(&mut self, _: &mut Self::Control) -> std::io::Result<compio_driver::Decision> {
        Ok(compio_driver::Decision::Blocking)
    }

    fn operate(&mut self, ctrl: &mut Self::Control) -> std::task::Poll<std::io::Result<usize>> {
        self.0.operate(ctrl)
    }

    unsafe fn set_result(
        &mut self,
        ctrl: &mut Self::Control,
        res: &std::io::Result<usize>,
        extra: &compio_driver::Extra,
    ) {
        unsafe { self.0.set_result(ctrl, res, extra) }
    }
}

/// A plain listener has no per-socket readiness state, so it is wrapped to
/// satisfy [`AsSocketState`].
struct Listener(TcpListener);

impl AsFd for Listener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsSocketState for Listener {}

fn push_and_wait<O: OpCode + 'static>(driver: &mut Proactor, op: O) -> BufResult<usize, O> {
    match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut key) => loop {
            driver.poll(None).unwrap();
            match driver.pop(key) {
                PushEntry::Pending(k) => key = k,
                PushEntry::Ready(res) => break res,
            }
        },
    }
}

/// One proactor per driver available at runtime: both under fusion, so that
/// each driver's blocking path is covered.
fn drivers() -> Vec<Proactor> {
    let mut drivers: Vec<Proactor> = Vec::new();
    for ty in [DriverType::IoUring, DriverType::Poll] {
        // A forced type is ignored outside fusion, and fails if unavailable.
        let Ok(driver) = Proactor::builder().driver_type(ty).build() else {
            continue;
        };
        if drivers
            .iter()
            .all(|d| d.driver_type() != driver.driver_type())
        {
            drivers.push(driver);
        }
    }
    assert!(!drivers.is_empty(), "no driver available");
    drivers
}

/// The result value must be the owned fd, and that fd must be open.
fn assert_owned_open(res: usize, fd: impl AsFd) {
    assert_eq!(res, fd.as_fd().as_raw_fd() as usize);
    rustix::io::fcntl_getfd(fd).expect("fd returned by the op must be open");
}

#[test]
fn open_file() {
    for mut driver in drivers() {
        let op = OpenFile::new(
            CurrentDir,
            CString::new("Cargo.toml").unwrap(),
            OFlags::CLOEXEC | OFlags::RDONLY,
            Mode::empty(),
        );
        let (res, op) = push_and_wait(&mut driver, ForceBlocking(op)).unwrap();
        let file = std::fs::File::from(op.0.into_inner());

        assert_owned_open(res, &file);
        let expected = std::fs::metadata("Cargo.toml").unwrap();
        let actual = file.metadata().unwrap();
        assert_eq!(
            (actual.dev(), actual.ino()),
            (expected.dev(), expected.ino())
        );
    }
}

#[test]
fn create_socket() {
    for mut driver in drivers() {
        let op = CreateSocket::new(libc::AF_INET, libc::SOCK_STREAM, 0);
        let (res, op) = push_and_wait(&mut driver, ForceBlocking(op)).unwrap();
        let socket = op.0.into_inner();

        assert_owned_open(res, &socket);
        assert_eq!(socket.r#type().unwrap(), socket2::Type::STREAM);
    }
}

#[test]
fn accept() {
    for mut driver in drivers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();

        let op = Accept::new(Listener(listener));
        let (res, op) = push_and_wait(&mut driver, ForceBlocking(op)).unwrap();
        let (socket, _) = op.0.into_inner();

        assert_owned_open(res, &socket);
        assert_eq!(
            socket.peer_addr().unwrap().as_socket(),
            Some(client.local_addr().unwrap())
        );
    }
}

#[test]
fn accept_multi() {
    for mut driver in drivers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();

        let op = AcceptMulti::new(Listener(listener));
        let (res, op) = push_and_wait(&mut driver, ForceBlocking(op)).unwrap();
        let socket = op.0.into_inner();

        assert_owned_open(res, &socket);
        assert_eq!(
            socket.peer_addr().unwrap().as_socket(),
            Some(client.local_addr().unwrap())
        );
    }
}
