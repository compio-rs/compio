use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
};

use crate::{impl_raw_fd, buf::BufWrapperMut, op::Recv, syscall, task::RUNTIME};


/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    sender: OwnedFd,
    receiver: OwnedFd,
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> io::Result<Self> {
        let (sender, receiver) = mio::unix::pipe::new()?;
        sender.set_nonblocking(false)?;
        let sender = unsafe { OwnedFd::from_raw_fd(sender.into_raw_fd()) };
        let receiver = unsafe { OwnedFd::from_raw_fd(receiver.into_raw_fd()) };
        Ok(Self { sender, receiver })
    }

    /// Get a notify handle.
    pub fn handle(&self) -> io::Result<EventHandle> {
        Ok(EventHandle::new(self.sender.try_clone()?))
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(&self) -> io::Result<()> {
        let buffer = Vec::with_capacity(1);
        // Trick: Recv uses readv which doesn't seek.
        let op = Recv::new(self.receiver.as_raw_fd(), BufWrapperMut::from(buffer));
        let (res, _) = RUNTIME.with(|runtime| runtime.submit(op)).await;
        res?;
        Ok(())
    }
}

impl AsRawFd for Event {
    fn as_raw_fd(&self) -> RawFd {
        self.receiver.as_raw_fd()
    }
}

/// A handle to [`Event`].
pub struct EventHandle {
    fd: OwnedFd,
}

impl EventHandle {
    pub(crate) fn new(fd: OwnedFd) -> Self {
        Self { fd }
    }

    /// Notify the event.
    pub fn notify(&self) -> io::Result<()> {
        let data = &[1];
        syscall!(write(self.fd.as_raw_fd(), data.as_ptr() as _, 1))?;
        Ok(())
    }
}

impl_raw_fd!(EventHandle, fd);
