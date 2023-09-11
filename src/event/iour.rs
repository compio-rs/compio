use std::{
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd},
};

use crate::{op::ReadAt, task::RUNTIME};

/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    fd: OwnedFd,
    registered_fd: RegisteredFd,
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> io::Result<Self> {
        let fd = unsafe { libc::eventfd(0, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let registered_fd = RUNTIME.with(|runtime| runtime.register_fd(fd))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { fd, registered_fd })
    }

    /// Get a notify handle.
    pub fn handle(&self) -> EventHandle {
        EventHandle::new(self.fd.as_fd())
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(&self) -> io::Result<()> {
        let buffer = Vec::with_capacity(8);
        let op = ReadAt::new(self.registered_fd, 0, buffer);
        let (res, _) = RUNTIME.with(|runtime| runtime.submit(op)).await;
        res?;
        Ok(())
    }
}

impl AsRegisteredFd for Event {
    fn as_registered_fd(&self) -> RegisteredFd {
        self.registered_fd
    }
}

/// A handle to [`Event`].
pub struct EventHandle<'a> {
    fd: BorrowedFd<'a>,
}

impl<'a> EventHandle<'a> {
    pub(crate) fn new(fd: BorrowedFd<'a>) -> Self {
        Self { fd }
    }

    /// Notify the event.
    pub fn notify(&self) -> io::Result<()> {
        let data = 1u64;
        let res = unsafe {
            libc::write(
                self.fd.as_raw_fd(),
                &data as *const _ as *const _,
                std::mem::size_of::<u64>(),
            )
        };
        if res < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
