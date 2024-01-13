use std::{io, pin::Pin, task::Poll};

use compio_driver::{Key, NotifyHandle, OpCode, PushEntry};
use windows_sys::Win32::System::IO::OVERLAPPED;

use crate::{runtime::op::OpFuture, Runtime};

/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    user_data: Key<NopPending>,
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> io::Result<Self> {
        let user_data = Runtime::current().inner().submit_raw(NopPending::new());
        let user_data = match user_data {
            PushEntry::Pending(user_data) => user_data,
            PushEntry::Ready(_) => unreachable!("NopPending always returns Pending"),
        };
        Ok(Self { user_data })
    }

    /// Get a notify handle.
    pub fn handle(&self) -> io::Result<EventHandle> {
        EventHandle::new(&self.user_data)
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(self) -> io::Result<()> {
        let future = OpFuture::new(self.user_data);
        future.await.0?;
        Ok(())
    }
}

/// A handle to [`Event`].
pub struct EventHandle {
    handle: NotifyHandle,
}

impl EventHandle {
    fn new(user_data: &Key<NopPending>) -> io::Result<Self> {
        let runtime = Runtime::current();
        Ok(Self {
            handle: unsafe { runtime.inner().handle_for(**user_data)? },
        })
    }

    /// Notify the event.
    pub fn notify(self) -> io::Result<()> {
        self.handle.notify()
    }
}

#[derive(Debug)]
struct NopPending {}

impl NopPending {
    pub fn new() -> Self {
        Self {}
    }
}

impl OpCode for NopPending {
    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Pending
    }

    unsafe fn cancel(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> io::Result<()> {
        Ok(())
    }
}
