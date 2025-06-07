use std::{io, process};

use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd,
    op::{Interest, PollOnce},
};

pub async fn child_wait(child: process::Child) -> io::Result<process::ExitStatus> {
    #[cfg(feature = "linux_pidfd")]
    let fd = {
        use std::os::linux::process::ChildExt;

        child.pidfd().ok().map(|fd| fd.as_raw_fd())
    };
    #[cfg(not(feature = "linux_pidfd"))]
    let fd = None::<RawFd>;
    if let Some(fd) = fd {
        struct PidFdWrap {
            child: process::Child,
            fd: RawFd,
        }

        impl AsRawFd for PidFdWrap {
            fn as_raw_fd(&self) -> RawFd {
                self.fd
            }
        }

        impl AsFd for PidFdWrap {
            fn as_fd(&self) -> BorrowedFd<'_> {
                unsafe { BorrowedFd::borrow_raw(self.fd) }
            }
        }

        // pidfd won't be closed, and the child has not been reaped.
        let fd = PidFdWrap { child, fd };
        let fd = SharedFd::new(fd);
        let op = PollOnce::new(fd.clone(), Interest::Readable);
        compio_runtime::submit(op).await.0?;
        let mut fd = fd.take().await.expect("cannot retrieve the child back");
        fd.child.wait()
    } else {
        unix::child_wait(child).await
    }
}

// For trait impls and fallback.
#[path = "unix.rs"]
mod unix;
