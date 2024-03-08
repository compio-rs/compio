use std::{io, os::fd::OwnedFd, process};

use compio_driver::{op::PollRead, syscall, AsRawFd, FromRawFd};
use compio_runtime::Runtime;

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    if let Ok(fd) = syscall!(libc::syscall(
        libc::SYS_pidfd_open,
        child.id() as libc::pid_t,
        0i32
    )) {
        let fd = unsafe { OwnedFd::from_raw_fd(fd as _) };
        let op = PollRead::new(fd.as_raw_fd());
        Runtime::current().submit(op).await.0?;
        child.wait()
    } else {
        unix::child_wait(child).await
    }
}

// For trait impls and fallback.
#[path = "unix.rs"]
mod unix;
