use crate::driver::{Entry, Poller};
use io_uring::{
    squeue,
    types::{SubmitArgs, Timespec},
    IoUring,
};
use std::{io, time::Duration};

pub use libc::{sockaddr_storage, socklen_t};
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

pub(crate) mod fs;
pub(crate) mod net;
pub(crate) mod op;

/// Abstraction of io-uring operations.
pub trait OpCode {
    /// Create submission entry.
    fn create_entry(&mut self) -> squeue::Entry;
}

/// Low-level driver of io-uring.
pub struct Driver {
    inner: IoUring,
}

impl Driver {
    /// Create a new io-uring driver with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(1024)
    }

    /// Create a new io-uring driver with specified entries.
    pub fn with_entries(entries: u32) -> io::Result<Self> {
        Ok(Self {
            inner: IoUring::new(entries)?,
        })
    }
}

impl Poller for Driver {
    fn attach(&self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    unsafe fn push(&self, op: &mut impl OpCode, user_data: usize) -> io::Result<()> {
        let entry = op.create_entry().user_data(user_data as _);
        if self.inner.submission_shared().is_full() {
            self.inner.submit()?;
        }
        self.inner.submission_shared().push(&entry).unwrap();
        Ok(())
    }

    fn poll(&self, timeout: Option<Duration>) -> io::Result<Entry> {
        if let Some(duration) = timeout {
            let timespec = timespec(duration);
            let args = SubmitArgs::new().timespec(&timespec);
            self.inner.submitter().submit_with_args(1, &args)?;
        } else {
            // Submit and Wait without timeout
            self.inner.submit_and_wait(1)?;
        }
        let entry = unsafe { self.inner.completion_shared() }.next().unwrap();
        let result = entry.result();
        let result = if result < 0 {
            Err(io::Error::from_raw_os_error(-result))
        } else {
            Ok(result as _)
        };
        Ok(Entry::new(entry.user_data() as _, result))
    }
}

fn timespec(duration: std::time::Duration) -> Timespec {
    Timespec::new()
        .sec(duration.as_secs())
        .nsec(duration.subsec_nanos())
}
