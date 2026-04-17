use std::io;
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

pub use libc::cmsghdr as CmsgHeader;
use rustix::{
    fs::{AtFlags, CWD, Stat},
    io::Errno,
};
use smallvec::SmallVec;

use crate::sys::prelude::*;

mod_use![stat, pipe, socket];

pub mod reexport {}

/// One item in local or more items on heap.
pub type Multi<T> = SmallVec<[T; 1]>;

/// The interest to poll a file descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interest {
    /// Represents a read operation.
    Readable,
    /// Represents a write operation.
    Writable,
}

/// A special file descriptor that always refers to the current working
/// directory. It represents [`AT_FDCWD`](libc::AT_FDCWD) in libc.
pub struct CurrentDir;

impl AsRawFd for CurrentDir {
    fn as_raw_fd(&self) -> RawFd {
        CWD.as_raw_fd()
    }
}

impl AsFd for CurrentDir {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(libc::AT_FDCWD) }
    }
}

/// Execute the function, retry on interruption, return [`Poll::Pending`] if
/// it's not finished yet, or return the result otherwise.
pub fn poll_io<R, E, F>(mut f: F) -> Poll<io::Result<R>>
where
    F: FnMut() -> std::result::Result<R, E>,
    E: Into<io::Error>,
{
    loop {
        match f().map_err(Into::into) {
            Ok(res) => break Poll::Ready(Ok(res)),
            Err(e) => match Errno::from_io_error(&e) {
                Some(Errno::WOULDBLOCK | Errno::INPROGRESS) => return Poll::Pending,
                Some(Errno::INTR) => continue,
                _ => return Poll::Ready(Err(e)),
            },
        }
    }
}
