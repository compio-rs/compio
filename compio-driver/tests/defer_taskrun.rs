#![cfg(io_uring)]

use std::{io::Write, os::unix::net::UnixStream, time::Duration};

use compio_buf::BufResult;
use compio_driver::{Proactor, PushEntry, SharedFd, op::Read};

/// `flush` on a DEFER_TASKRUN ring runs the task work of an op that became
/// ready after it was submitted, so the op reads its data without a wait.
/// Skipped when io_uring or DEFER_TASKRUN (Linux 6.1) is unavailable.
#[test]
fn defer_taskrun_flush_runs_task_work() {
    let mut builder = Proactor::builder();
    builder.single_issuer(true).defer_taskrun(true);
    let Ok(mut driver) = builder.build() else {
        return;
    };
    if !driver.driver_type().is_iouring() {
        return;
    }

    let (rx, mut tx) = UnixStream::pair().unwrap();
    let rx = SharedFd::new(rx);
    let key = match driver.push(Read::new(rx.clone(), Vec::with_capacity(8))) {
        PushEntry::Pending(key) => key,
        PushEntry::Ready(res) => panic!("read completed before data was sent: {:?}", res.0),
    };
    driver.flush();

    tx.write_all(b"ping").unwrap();
    driver.flush();
    assert_eq!(rustix::io::ioctl_fionread(&rx).unwrap(), 0);

    driver.poll(Some(Duration::ZERO)).unwrap();
    let BufResult(res, _) = match driver.pop(key) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(_) => panic!("read did not complete"),
    };
    assert_eq!(res.unwrap(), 4);
}
