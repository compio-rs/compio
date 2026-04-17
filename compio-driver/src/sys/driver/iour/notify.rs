use rustix::event::{EventfdFlags, eventfd};

use super::*;

#[derive(Debug)]
pub(super) struct Notifier {
    notify: Arc<Notify>,
}

impl Notifier {
    /// Create a new notifier.
    pub fn new() -> io::Result<Self> {
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;

        Ok(Self {
            notify: Arc::new(Notify::new(fd)),
        })
    }

    pub fn clear(&self) -> io::Result<()> {
        const LEN: usize = std::mem::size_of::<u64>();

        let mut buffer = [0u8; LEN];

        let res = poll_io(|| rustix::io::read(self, &mut buffer))?;

        debug_assert!(matches!(res, Poll::Pending | Poll::Ready(LEN)));

        Ok(())
    }

    pub fn waker(&self) -> Waker {
        Waker::from(self.notify.clone())
    }
}

impl AsFd for Notifier {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.notify.fd.as_fd()
    }
}

impl AsRawFd for Notifier {
    fn as_raw_fd(&self) -> RawFd {
        self.notify.fd.as_raw_fd()
    }
}

/// A notify handle to the inner driver.
#[derive(Debug)]
pub(super) struct Notify {
    fd: OwnedFd,
}

impl Notify {
    pub fn new(fd: OwnedFd) -> Self {
        Self { fd }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        rustix::io::write(&self.fd, &u64::to_be_bytes(1))?;

        Ok(())
    }
}

impl Wake for Notify {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify().ok();
    }
}
