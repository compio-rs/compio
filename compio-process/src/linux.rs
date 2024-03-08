use std::{io, process};

use compio_driver::op::PollRead;
use compio_runtime::Runtime;

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    #[cfg(feature = "linux_pidfd")]
    let fd = {
        use std::os::linux::process::ChildExt;

        use compio_driver::AsRawFd;

        child.pidfd().ok().map(|fd| fd.as_raw_fd())
    };
    #[cfg(not(feature = "linux_pidfd"))]
    let fd = None;
    if let Some(fd) = fd {
        let op = PollRead::new(fd);
        Runtime::current().submit(op).await.0?;
        child.wait()
    } else {
        unix::child_wait(child).await
    }
}

// For trait impls and fallback.
#[path = "unix.rs"]
mod unix;
