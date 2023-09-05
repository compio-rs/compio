//! Windows-specific types for signal handling.

use std::{
    collections::HashMap,
    future::Future,
    io,
    pin::Pin,
    sync::{Mutex, Once},
    task::{Context, Poll},
};

use once_cell::sync::Lazy as LazyLock;
use slab::Slab;
use windows_sys::Win32::{
    Foundation::BOOL,
    System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
        CTRL_SHUTDOWN_EVENT,
    },
};

use crate::{
    driver::{Driver, Poller},
    task::RUNTIME,
};

#[allow(clippy::type_complexity)]
static HANDLER: LazyLock<Mutex<HashMap<u32, Slab<Box<dyn FnOnce() + Send + Sync>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

unsafe extern "system" fn ctrl_event_handler(ctrltype: u32) -> BOOL {
    let mut handler = HANDLER.lock().unwrap();
    if let Some(handlers) = handler.get_mut(&ctrltype) {
        if !handlers.is_empty() {
            let handlers = std::mem::replace(handlers, Slab::new());
            for (_, handler) in handlers {
                handler();
            }
            return 1;
        }
    }
    0
}

static INIT: Once = Once::new();

fn init() -> io::Result<()> {
    let res = unsafe { SetConsoleCtrlHandler(Some(ctrl_event_handler), 1) };
    if res == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn register(ctrltype: u32, f: impl FnOnce() + Send + Sync + 'static) -> usize {
    let mut handler = HANDLER.lock().unwrap();
    handler.entry(ctrltype).or_default().insert(Box::new(f))
}

fn unregister(ctrltype: u32, key: usize) {
    let mut handler = HANDLER.lock().unwrap();
    if let Some(handlers) = handler.get_mut(&ctrltype) {
        if handlers.contains(key) {
            let _ = handlers.remove(key);
        }
    }
}

/// Represents a listener to console CTRL event.
#[derive(Debug)]
pub struct CtrlEvent {
    ctrltype: u32,
    user_data: usize,
    handler_key: usize,
}

impl CtrlEvent {
    pub(crate) fn new(ctrltype: u32) -> Self {
        INIT.call_once(|| init().unwrap());

        let user_data = RUNTIME.with(|runtime| runtime.submit_dummy());
        let handler_key = RUNTIME.with(|runtime| {
            // Safety: the runtime is thread-local static, and the driver is send & sync.
            let driver = unsafe {
                (runtime.driver() as *const Driver)
                    .as_ref()
                    .unwrap_unchecked()
            };
            register(ctrltype, move || driver.post(user_data, 0).unwrap())
        });
        Self {
            ctrltype,
            user_data,
            handler_key,
        }
    }
}

impl Future for CtrlEvent {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        RUNTIME
            .with(|runtime| runtime.poll_dummy(cx, self.user_data))
            .map(|res| res.map(|_| ()))
    }
}

impl Drop for CtrlEvent {
    fn drop(&mut self) {
        unregister(self.ctrltype, self.handler_key);
    }
}

/// Creates a new listener which receives "ctrl-break" notifications sent to the
/// process.
pub fn ctrl_break() -> CtrlEvent {
    CtrlEvent::new(CTRL_BREAK_EVENT)
}

/// Creates a new listener which receives "ctrl-close" notifications sent to the
/// process.
pub fn ctrl_close() -> CtrlEvent {
    CtrlEvent::new(CTRL_CLOSE_EVENT)
}

/// Creates a new listener which receives "ctrl-c" notifications sent to the
/// process.
pub fn ctrl_c() -> CtrlEvent {
    CtrlEvent::new(CTRL_C_EVENT)
}

/// Creates a new listener which receives "ctrl-logoff" notifications sent to
/// the process.
pub fn ctrl_logoff() -> CtrlEvent {
    CtrlEvent::new(CTRL_LOGOFF_EVENT)
}

/// Creates a new listener which receives "ctrl-shutdown" notifications sent to
/// the process.
pub fn ctrl_shutdown() -> CtrlEvent {
    CtrlEvent::new(CTRL_SHUTDOWN_EVENT)
}
