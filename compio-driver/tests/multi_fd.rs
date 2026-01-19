#![cfg(all(not(io_uring), unix))]

use std::{os::fd::RawFd, pin::Pin, task::Poll};

use compio_driver::{Decision, OpCode, OpType, WaitArg};

struct TwoFd(RawFd, RawFd);

impl OpCode for TwoFd {
    fn pre_submit(self: Pin<&mut Self>) -> std::io::Result<Decision> {
        Ok(Decision::wait_for_many([
            WaitArg::readable(self.0),
            WaitArg::writable(self.1),
        ]))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::multi_fd([self.0, self.1]))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Ok(42))
    }
}

#[test]
fn multi_fd_op() {
    use compio_driver::{Proactor, PushEntry};

    let mut driver = Proactor::new().unwrap();

    // Use stdin and stdout as dummy fds.
    let op = TwoFd(0, 1);

    match driver.push(op) {
        PushEntry::Ready(res) => {
            let (val, _) = res.unwrap();
            assert_eq!(val, 42);
        }
        PushEntry::Pending(mut user_data) => loop {
            driver.poll(None).unwrap();
            match driver.pop(user_data) {
                PushEntry::Pending(k) => user_data = k,
                PushEntry::Ready(res) => {
                    let (val, _) = res.unwrap();
                    assert_eq!(val, 42);
                    break;
                }
            }
        },
    }
}
