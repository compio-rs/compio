use std::{io, marker::PhantomData};

use crate::{
    driver::{post_driver, AsRawFd, RawFd},
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
pub struct EventHandle<'a> {
    user_data: usize,
    handle: RawFd,
    _p: PhantomData<&'a Event>,
}

unsafe impl Send for EventHandle<'_> {}
unsafe impl Sync for EventHandle<'_> {}

impl<'a> EventHandle<'a> {
    pub(crate) fn new(user_data: &'a Key<()>) -> Self {
        let handle = RUNTIME.with(|runtime| runtime.driver().as_raw_fd());
        Self {
            user_data: **user_data,
            handle,
            _p: PhantomData,
        }
    }

    /// Notify the event.
    pub fn notify(&self) -> io::Result<()> {
        post_driver(self.handle, self.user_data, Ok(0))
    }
}
