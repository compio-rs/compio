use std::io;

use crate::{
    driver::{post_driver, RawFd},
    key::Key,
    task::{op::OpFuture, RUNTIME},
};

/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    user_data: Key<()>,
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> io::Result<Self> {
        let user_data = RUNTIME.with(|runtime| runtime.submit_dummy());
        Ok(Self { user_data })
    }

    /// Get a notify handle.
    pub fn handle(&self) -> EventHandle {
        EventHandle::new(&self.user_data)
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(&self) -> io::Result<()> {
        let future = OpFuture::new(self.user_data);
        future.await?;
        Ok(())
    }
}

/// A handle to [`Event`].
pub struct EventHandle {
    user_data: usize,
    handle: RawFd,
}

// Safety: IOCP handle is thread safe.
unsafe impl Send for EventHandle {}
unsafe impl Sync for EventHandle {}

impl EventHandle {
    pub(crate) fn new(user_data: &Key<()>) -> Self {
        let handle = RUNTIME.with(|runtime| runtime.raw_driver());
        Self {
            user_data: **user_data,
            handle,
        }
    }

    /// Notify the event.
    pub fn notify(&self) -> io::Result<()> {
        post_driver(self.handle, self.user_data, Ok(0))
    }
}
