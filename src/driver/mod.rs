use std::{io, task::Poll, time::Duration};

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    }
}

pub trait Poller {
    fn attach(&self, fd: RawFd) -> io::Result<()>;

    fn submit(&self, op: impl OpCode, user_data: usize) -> Poll<io::Result<usize>>;

    fn poll(&self, timeout: Option<Duration>) -> io::Result<Entry>;
}

pub struct Entry {
    user_data: usize,
    result: io::Result<usize>,
}

impl Entry {
    pub fn user_data(&self) -> usize {
        self.user_data
    }

    pub fn into_result(self) -> io::Result<usize> {
        self.result
    }
}
