use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    time::Duration,
};

use compio_driver::AsRawFd;
use compio_runtime::Runtime;
use windows_sys::Win32::{
    Foundation::{WAIT_FAILED, WAIT_TIMEOUT},
    System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
};

use crate::sys::Adapter;

struct WindowsAdapter {
    runtime: Runtime,
}

impl WindowsAdapter {
    fn new(runtime: Runtime) -> io::Result<Self> {
        Ok(Self { runtime })
    }

    async fn wait(&self, timeout: Option<Duration>) -> io::Result<()> {
        let (sender, receiver) = oneshot::async_channel::<io::Result<()>>();
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let event_handle = unsafe { OwnedHandle::from_raw_handle(event as RawHandle) };

        let timeout = match timeout {
            Some(timeout) => timeout.as_millis() as u32,
            None => INFINITE,
        };

        struct EventGuard(OwnedHandle);

        impl Drop for EventGuard {
            fn drop(&mut self) {
                unsafe { SetEvent(self.0.as_raw_handle()) };
            }
        }

        let _event_handle = EventGuard(event_handle);
        let event = event as usize;
        let driver = self.runtime.as_raw_fd() as usize;
        windows_threading::submit(move || {
            let handles = [event as RawHandle, driver as RawHandle];
            let res = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout) };
            let res = match res {
                WAIT_FAILED => Err(io::Error::last_os_error()),
                WAIT_TIMEOUT => Err(io::ErrorKind::TimedOut.into()),
                _ => Ok(()),
            };
            sender.send(res).ok();
        });
        receiver
            .await
            .map_err(|_| io::ErrorKind::Interrupted.into())
            .flatten()
    }
}

macro_rules! impl_adapter {
    ($name:ident) => {
        pub struct $name(WindowsAdapter);

        impl Adapter for $name {
            fn new(runtime: Runtime) -> io::Result<Self> {
                WindowsAdapter::new(runtime).map(Self)
            }

            async fn wait(&self, timeout: Option<Duration>) -> io::Result<()> {
                self.0.wait(timeout).await
            }

            fn clear(&self) -> io::Result<()> {
                Ok(())
            }
        }

        impl std::ops::Deref for $name {
            type Target = Runtime;

            fn deref(&self) -> &Self::Target {
                &self.0.runtime
            }
        }
    };
}

#[cfg(feature = "tokio")]
impl_adapter!(TokioAdapter);

#[cfg(feature = "futures")]
impl_adapter!(FuturesAdapter);
