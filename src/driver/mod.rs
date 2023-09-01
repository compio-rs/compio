//! The platform-specified driver.
//! Some types differ by compilation target.

use std::{io, time::Duration};

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(target_os = "linux")] {
        mod iour;
        pub use iour::*;
    }
}

/// An abstract of [`Driver`].
/// It contains some low-level actions of completion-based IO.
///
/// You don't need them unless you are controlling a [`Driver`] yourself.
pub trait Poller {
    /// Attach an fd to the driver.
    fn attach(&self, fd: RawFd) -> io::Result<()>;

    /// Push an operation with user-defined data.
    /// The data could be retrived from [`Entry`] when polling.
    ///
    /// # Safety
    ///
    /// `op` should be alive until [`Poller::poll`] returns its result.
    unsafe fn push(&self, op: &mut (impl OpCode + 'static), user_data: usize) -> io::Result<()>;

    /// Poll the driver with an optional timeout.
    /// If no timeout specified, the call will block.
    fn poll(&self, timeout: Option<Duration>) -> io::Result<Entry>;
}

/// An completed entry returned from kernel.
pub struct Entry {
    user_data: usize,
    result: io::Result<usize>,
}

impl Entry {
    pub(crate) fn new(user_data: usize, result: io::Result<usize>) -> Self {
        Self { user_data, result }
    }

    /// The user-defined data passed to [`Poller::push`].
    pub fn user_data(&self) -> usize {
        self.user_data
    }

    /// The result of the operation.
    pub fn into_result(self) -> io::Result<usize> {
        self.result
    }
}
