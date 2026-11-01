//! Ops that create an fd must hand back an open fd when io_uring runs them on
//! the blocking thread pool (opcode unsupported by the running kernel).

#![cfg(all(io_uring, not(fusion)))]

use std::{
    ffi::CString,
    io,
    net::{TcpListener, TcpStream},
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    Extra, OpCode, OpEntry, Proactor, PushEntry,
    op::{Accept, CreateSocket, CurrentDir, Mode, OFlags, OpenFile},
};

/// Forces the blocking path, as `Driver::push` does for unsupported opcodes.
struct ForceBlocking<T>(T);

unsafe impl<T: OpCode> OpCode for ForceBlocking<T> {
    type Control = T::Control;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.0.init(ctrl) }
    }

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, ctrl: &mut Self::Control) -> io::Result<usize> {
        self.0.call_blocking(ctrl)
    }

    unsafe fn set_result(
        &mut self,
        ctrl: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &Extra,
    ) {
        unsafe { self.0.set_result(ctrl, res, extra) }
    }
}

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

/// The result value must be the owned fd, and that fd must be open.
fn assert_owned_open(res: usize, fd: impl AsFd) {
    assert_eq!(res, fd.as_fd().as_raw_fd() as usize);
    rustix::io::fcntl_getfd(fd).expect("fd returned by the op must be open");
}

#[test]
fn open_file() {
    let mut driver = Proactor::new().unwrap();
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

#[test]
fn create_socket() {
    let mut driver = Proactor::new().unwrap();
    let op = CreateSocket::new(libc::AF_INET, libc::SOCK_STREAM, 0);
    let (res, op) = push_and_wait(&mut driver, ForceBlocking(op)).unwrap();
    let socket = op.0.into_inner();

    assert_owned_open(res, &socket);
    assert_eq!(socket.r#type().unwrap(), socket2::Type::STREAM);
}

#[test]
fn accept() {
    let mut driver = Proactor::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();

    let op = Accept::new(listener);
    let (res, op) = push_and_wait(&mut driver, ForceBlocking(op)).unwrap();
    let (socket, _) = op.0.into_inner();

    assert_owned_open(res, &socket);
    assert_eq!(
        socket.peer_addr().unwrap().as_socket(),
        Some(client.local_addr().unwrap())
    );
}
