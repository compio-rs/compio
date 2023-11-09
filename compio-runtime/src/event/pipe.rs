use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
};

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_driver::{impl_raw_fd, op::Recv, syscall};

use crate::{attacher::Attacher, submit};

/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    sender: OwnedFd,
    receiver: OwnedFd,
    attacher: Attacher,
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> io::Result<Self> {
        let (receiver, sender) = os_pipe::pipe()?;
        let receiver = unsafe { OwnedFd::from_raw_fd(receiver.into_raw_fd()) };
        let sender = unsafe { OwnedFd::from_raw_fd(sender.into_raw_fd()) };

        syscall!(libc::fcntl(
            receiver.as_raw_fd(),
            libc::F_SETFL,
            libc::O_NONBLOCK
        ))?;
        Ok(Self {
            sender,
            receiver,
            attacher: Attacher::new(),
        })
    }

    /// Get a notify handle.
    pub fn handle(&self) -> io::Result<EventHandle> {
        Ok(EventHandle::new(self.sender.try_clone()?))
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(self) -> io::Result<()> {
        self.attacher.attach(&self.receiver)?;
        let buffer = ArrayVec::<u8, 1>::new();
        // Trick: Recv uses readv which doesn't seek.
        let op = Recv::new(self.receiver.as_raw_fd(), buffer);
        let BufResult(res, _) = submit(op).await;
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
    pub fn notify(self) -> io::Result<()> {
        let data = &[1];
        syscall!(libc::write(self.fd.as_raw_fd(), data.as_ptr() as _, 1))?;
        Ok(())
    }
}

impl_raw_fd!(EventHandle, fd);
