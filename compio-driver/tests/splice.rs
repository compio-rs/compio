#![cfg(unix)]

use std::os::fd::OwnedFd;

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsRawFd, OpCode, Proactor, PushEntry, SharedFd,
    op::{Pipe, Read, Write},
};

#[cfg(not(linux_all))]
mod splice_impl {
    use std::{
        io,
        os::fd::{AsFd, AsRawFd},
        task::{Poll, ready},
    };

    use compio_driver::{Decision, OpCode, OpType, WaitArg, syscall};

    pub struct Splice<S1, S2> {
        pub(crate) fd_in: S1,
        pub(crate) fd_out: S2,
        pub(crate) len: usize,
    }

    pub fn splice<S1, S2>(fd_in: S1, fd_out: S2, len: usize) -> Splice<S1, S2> {
        Splice { fd_in, fd_out, len }
    }

    unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
        type Control = ();

        unsafe fn init(&mut self, _: &mut Self::Control) {}

        fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
            Ok(Decision::wait_for_many([
                WaitArg::readable(self.fd_in.as_fd().as_raw_fd()),
                WaitArg::writable(self.fd_out.as_fd().as_raw_fd()),
            ]))
        }

        fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
            Some(OpType::multi_fd([
                self.fd_in.as_fd().as_raw_fd(),
                self.fd_out.as_fd().as_raw_fd(),
            ]))
        }

        fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
            let mut buffer = vec![0u8; self.len];
            ready!(syscall!(
                break libc::read(
                    self.fd_in.as_fd().as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len()
                )
            ))?;
            // Cannot return pending here because the data has been read already.
            Poll::Ready(syscall!(libc::write(
                self.fd_out.as_fd().as_raw_fd(),
                buffer.as_ptr().cast(),
                buffer.len()
            )))
        }
    }
}

#[cfg(linux_all)]
mod splice_impl {
    use compio_driver::op::{Splice, SpliceFlags};

    pub fn splice<S1, S2>(fd_in: S1, fd_out: S2, len: usize) -> Splice<S1, S2> {
        Splice::new(fd_in, -1, fd_out, -1, len, SpliceFlags::NONBLOCK)
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

fn pipe2(driver: &mut Proactor) -> (OwnedFd, OwnedFd) {
    let op = Pipe::new();
    let (_, op) = push_and_wait(driver, op).unwrap();
    op.into_inner()
}

#[test]
fn splice() {
    let mut driver = Proactor::new().unwrap();

    let (rx, tx) = pipe2(&mut driver);
    let (rx1, tx1) = pipe2(&mut driver);
    println!(
        "rx={}, tx={}, rx1={}, tx1={}",
        rx.as_raw_fd(),
        tx.as_raw_fd(),
        rx1.as_raw_fd(),
        tx1.as_raw_fd()
    );

    let write_op = Write::new(SharedFd::new(tx), b"hello world");
    let read_op = Read::new(SharedFd::new(rx1), Vec::with_capacity(11));
    let splice_op = splice_impl::splice(SharedFd::new(rx), SharedFd::new(tx1), 11);

    push_and_wait(&mut driver, write_op).unwrap();
    push_and_wait(&mut driver, splice_op).unwrap();
    let (res, op) = push_and_wait(&mut driver, read_op).unwrap();
    let mut buf = op.into_inner();
    assert_eq!(res, 11);
    unsafe { buf.set_len(res) }
    assert_eq!(&buf[..], b"hello world");
}
