#![cfg(unix)]

#[cfg(not(any(freebsd, solarish, linux_all)))]
use std::os::unix::io::OwnedFd;

use compio_buf::{BufResult, IntoInner};
#[cfg(linux_all)]
use compio_driver::op::Splice;
use compio_driver::{
    AsRawFd, OpCode, Proactor, PushEntry, SharedFd,
    op::{Read, Write},
};
#[cfg(linux_all)]
use libc::SPLICE_F_NONBLOCK;
use nix::fcntl::OFlag;
#[cfg(any(freebsd, solarish, linux_all))]
use nix::unistd::pipe2;

#[cfg(not(any(freebsd, solarish, linux_all)))]
fn pipe2(flags: OFlag) -> nix::Result<(OwnedFd, OwnedFd)> {
    use nix::{
        fcntl::{F_GETFD, F_GETFL, F_SETFD, F_SETFL, FdFlag, fcntl},
        unistd::pipe,
    };

    fn set_cloexec(fd: &OwnedFd) -> nix::Result<()> {
        let flag = FdFlag::from_bits_retain(fcntl(fd, F_GETFD)?);
        fcntl(fd, F_SETFD(flag | FdFlag::FD_CLOEXEC))?;
        Ok(())
    }

    fn set_nonblock(fd: &OwnedFd) -> nix::Result<()> {
        let flag = OFlag::from_bits_retain(fcntl(fd, F_GETFL)?);
        fcntl(fd, F_SETFL(flag | OFlag::O_NONBLOCK))?;
        Ok(())
    }

    let (r, w) = pipe()?;
    if flags.contains(OFlag::O_CLOEXEC) {
        set_cloexec(&r)?;
        set_cloexec(&w)?;
    }
    if flags.contains(OFlag::O_NONBLOCK) {
        set_nonblock(&r)?;
        set_nonblock(&w)?;
    }
    Ok((r, w))
}

#[cfg(not(linux_all))]
mod splice_impl {
    use std::{
        io,
        os::fd::{AsFd, AsRawFd},
        pin::Pin,
        task::{Poll, ready},
    };

    use compio_driver::{Decision, OpCode, OpType, WaitArg, syscall};

    pub const SPLICE_F_NONBLOCK: u32 = 0;

    pub struct Splice<S1, S2> {
        pub(crate) fd_in: S1,
        pub(crate) fd_out: S2,
        pub(crate) len: usize,
    }

    impl<S1, S2> Splice<S1, S2> {
        pub fn new(
            fd_in: S1,
            _offset_in: i64,
            fd_out: S2,
            _offset_out: i64,
            len: usize,
            _flags: u32,
        ) -> Self {
            Self { fd_in, fd_out, len }
        }
    }

    unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
        fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
            Ok(Decision::wait_for_many([
                WaitArg::readable(self.fd_in.as_fd().as_raw_fd()),
                WaitArg::writable(self.fd_out.as_fd().as_raw_fd()),
            ]))
        }

        fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
            Some(OpType::multi_fd([
                self.fd_in.as_fd().as_raw_fd(),
                self.fd_out.as_fd().as_raw_fd(),
            ]))
        }

        fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
            let mut buffer = vec![0u8; self.len];
            ready!(syscall!(
                break libc::read(
                    self.fd_in.as_fd().as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len()
                )
            ))?;
            // Cannot return pending here because the data has been read already.
            Poll::Ready(
                syscall!(libc::write(
                    self.fd_out.as_fd().as_raw_fd(),
                    buffer.as_ptr().cast(),
                    buffer.len()
                ))
                .map(|n| n as usize),
            )
        }
    }
}

#[cfg(not(linux_all))]
use splice_impl::{SPLICE_F_NONBLOCK, Splice};

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

#[test]
fn splice() {
    let mut driver = Proactor::new().unwrap();

    let mut flags = OFlag::O_CLOEXEC;
    if driver.driver_type().is_polling() {
        flags |= OFlag::O_NONBLOCK;
    }

    let (rx, tx) = pipe2(flags).unwrap();
    let (rx1, tx1) = pipe2(flags).unwrap();
    println!(
        "rx={}, tx={}, rx1={}, tx1={}",
        rx.as_raw_fd(),
        tx.as_raw_fd(),
        rx1.as_raw_fd(),
        tx1.as_raw_fd()
    );

    let write_op = Write::new(SharedFd::new(tx), b"hello world");
    let read_op = Read::new(SharedFd::new(rx1), Vec::with_capacity(11));
    let splice_op = Splice::new(
        SharedFd::new(rx),
        -1,
        SharedFd::new(tx1),
        -1,
        11,
        SPLICE_F_NONBLOCK,
    );

    push_and_wait(&mut driver, write_op).unwrap();
    push_and_wait(&mut driver, splice_op).unwrap();
    let (res, op) = push_and_wait(&mut driver, read_op).unwrap();
    let mut buf = op.into_inner();
    assert_eq!(res, 11);
    unsafe { buf.set_len(res) }
    assert_eq!(&buf[..], b"hello world");
}
