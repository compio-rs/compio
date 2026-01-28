use std::panic::resume_unwind;

use compio_runtime::event::Event;

#[test]
fn event_handle() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let event = Event::new();
        let handle = event.handle();
        let task = compio_runtime::spawn_blocking(move || {
            handle.notify();
        });
        event.wait().await;
        task.await.unwrap_or_else(|e| resume_unwind(e));
    })
}

#[test]
#[cfg(windows)]
fn win32_event() {
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        pin::Pin,
        ptr::null,
        task::Poll,
    };

    use compio_driver::{OpCode, OpType};
    use windows_sys::Win32::System::{
        IO::OVERLAPPED,
        Threading::{CreateEventW, SetEvent},
    };

    struct WaitEvent {
        event: OwnedHandle,
    }

    unsafe impl OpCode for WaitEvent {
        fn op_type(&self) -> OpType {
            OpType::Event(self.event.as_raw_handle() as _)
        }

        unsafe fn operate(
            self: Pin<&mut Self>,
            _optr: *mut OVERLAPPED,
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(0))
        }
    }

    compio_runtime::Runtime::new().unwrap().block_on(async {
        let event = unsafe { CreateEventW(null(), 0, 0, null()) };
        if event.is_null() {
            panic!("{:?}", std::io::Error::last_os_error());
        }
        let event = unsafe { OwnedHandle::from_raw_handle(event as _) };

        let event_raw = event.as_raw_handle() as isize;

        let wait = compio_runtime::submit(WaitEvent { event });

        let task = compio_runtime::spawn_blocking(move || {
            unsafe { SetEvent(event_raw as _) };
        });

        wait.await.0.unwrap();
        task.await.unwrap_or_else(|e| resume_unwind(e));
    })
}
