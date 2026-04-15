use std::os::windows::prelude::*;

use windows_sys::Win32::Foundation::HANDLE;

/// On windows, handle and socket are in the same size.
/// Both of them could be attached to an IOCP.
/// Therefore, both could be seen as fd.
pub type RawFd = HANDLE;

pub use std::os::windows::io::{FromRawHandle as FromRawFd, IntoRawHandle as IntoRawFd};

/// Extracts raw fds.
pub trait AsRawFd {
    /// Extracts the raw fd.
    fn as_raw_fd(&self) -> RawFd;
}

/// Owned handle or socket on Windows.
#[derive(Debug)]
pub enum OwnedFd {
    /// Win32 handle.
    File(OwnedHandle),
    /// Windows socket handle.
    Socket(OwnedSocket),
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(fd) => fd.as_raw_handle() as _,
            Self::Socket(s) => s.as_raw_socket() as _,
        }
    }
}

impl AsRawFd for RawFd {
    fn as_raw_fd(&self) -> RawFd {
        *self
    }
}

impl AsRawFd for std::fs::File {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for OwnedHandle {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for socket2::Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl AsRawFd for OwnedSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl AsRawFd for std::process::ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for std::process::ChildStdout {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for std::process::ChildStderr {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl From<OwnedHandle> for OwnedFd {
    fn from(value: OwnedHandle) -> Self {
        Self::File(value)
    }
}

impl From<std::fs::File> for OwnedFd {
    fn from(value: std::fs::File) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStdin> for OwnedFd {
    fn from(value: std::process::ChildStdin) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStdout> for OwnedFd {
    fn from(value: std::process::ChildStdout) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStderr> for OwnedFd {
    fn from(value: std::process::ChildStderr) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<OwnedSocket> for OwnedFd {
    fn from(value: OwnedSocket) -> Self {
        Self::Socket(value)
    }
}

impl From<socket2::Socket> for OwnedFd {
    fn from(value: socket2::Socket) -> Self {
        Self::Socket(OwnedSocket::from(value))
    }
}

/// Borrowed handle or socket on Windows.
#[derive(Debug)]
pub enum BorrowedFd<'a> {
    /// Win32 handle.
    File(BorrowedHandle<'a>),
    /// Windows socket handle.
    Socket(BorrowedSocket<'a>),
}

impl AsRawFd for BorrowedFd<'_> {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(fd) => fd.as_raw_handle() as RawFd,
            Self::Socket(s) => s.as_raw_socket() as RawFd,
        }
    }
}

impl<'a> From<BorrowedHandle<'a>> for BorrowedFd<'a> {
    fn from(value: BorrowedHandle<'a>) -> Self {
        Self::File(value)
    }
}

impl<'a> From<BorrowedSocket<'a>> for BorrowedFd<'a> {
    fn from(value: BorrowedSocket<'a>) -> Self {
        Self::Socket(value)
    }
}

/// Extracts fds.
pub trait AsFd {
    /// Extracts the borrowed fd.
    fn as_fd(&self) -> BorrowedFd<'_>;
}

impl AsFd for OwnedFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::File(fd) => fd.as_fd(),
            Self::Socket(s) => s.as_fd(),
        }
    }
}

impl AsFd for std::fs::File {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for OwnedHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for BorrowedHandle<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        (*self).into()
    }
}

impl AsFd for socket2::Socket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_socket().into()
    }
}

impl AsFd for OwnedSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_socket().into()
    }
}

impl AsFd for BorrowedSocket<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        (*self).into()
    }
}

impl AsFd for std::process::ChildStdin {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for std::process::ChildStdout {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for std::process::ChildStderr {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}
