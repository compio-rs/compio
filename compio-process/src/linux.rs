use std::{io, process};

use compio_driver::{
    op::{Interest, PollOnce},
    SharedFd,
};

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    #[cfg(feature = "linux_pidfd")]
    let fd = {
        use std::os::linux::process::ChildExt;

        use compio_driver::AsRawFd;

        child.pidfd().ok().map(|fd| fd.as_raw_fd())
    };
    #[cfg(not(feature = "linux_pidfd"))]
    let fd = None::<compio_driver::RawFd>;
    if let Some(fd) = fd {
        // pidfd won't be closed, and the child has not been reaped.
        let fd = SharedFd::new(fd);
        let op = PollOnce::new(fd, Interest::Readable);
        compio_runtime::submit(op).await.0?;
        child.wait()
    } else {
        unix::child_wait(child).await
    }
}

// For trait impls and fallback.
#[path = "unix.rs"]
mod unix;
