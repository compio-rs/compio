use std::{io, pin::Pin, ptr::null_mut, task::Poll};

use compio_driver::{syscall, OpCode, RawFd};
use windows_sys::Win32::System::IO::{PostQueuedCompletionStatus, OVERLAPPED};

use crate::{key::Key, runtime::op::OpFuture, RUNTIME};

/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    user_data: Key<NopPending>,
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> io::Result<Self> {
        let user_data = RUNTIME.with(|runtime| runtime.submit_raw(NopPending::new()));
        Ok(Self { user_data })
    }

    /// Get a notify handle.
    pub fn handle(&self) -> io::Result<EventHandle> {
        Ok(EventHandle::new(&self.user_data))
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(&self) -> io::Result<()> {
        let future = OpFuture::new(self.user_data);
        future.await.0?;
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
    fn new(user_data: &Key<NopPending>) -> Self {
        let handle = RUNTIME.with(|runtime| runtime.raw_driver());
        Self {
            user_data: **user_data,
            handle,
        }
    }

    /// Notify the event.
    pub fn notify(&self) -> io::Result<()> {
        post_driver_nop(self.handle, self.user_data)
    }
}

fn post_driver_nop(handle: RawFd, user_data: usize) -> io::Result<()> {
    syscall!(
        BOOL,
        PostQueuedCompletionStatus(handle as _, 0, user_data, null_mut())
    )?;
    Ok(())
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
