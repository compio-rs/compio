use std::{io, os::fd::OwnedFd, process};

use compio_driver::{op::PollRead, AsRawFd, FromRawFd};
use compio_runtime::Runtime;

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    #[cfg(feature = "linux_pidfd")]
    let fd = {
        use std::os::linux::process::ChildExt;
        child.pidfd().ok().map(|fd| fd.as_raw_fd())
    };
    #[cfg(not(feature = "linux_pidfd"))]
    let fd: Option<RawFd> = None;
    if let Some(fd) = fd {
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
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
