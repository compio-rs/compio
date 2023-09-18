//! Windows-specific types for signal handling.

#[cfg(feature = "lazy_cell")]
use std::sync::LazyLock;
use std::{
    collections::HashMap,
    io,
    sync::{Mutex, Once},
};

#[cfg(not(feature = "lazy_cell"))]
use once_cell::sync::Lazy as LazyLock;
use slab::Slab;
use windows_sys::Win32::{
    Foundation::BOOL,
    System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
        CTRL_SHUTDOWN_EVENT,
    },
};

use crate::event::{Event, EventHandle};

static HANDLER: LazyLock<Mutex<HashMap<u32, Slab<EventHandle>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

unsafe extern "system" fn ctrl_event_handler(ctrltype: u32) -> BOOL {
    let mut handler = HANDLER.lock().unwrap();
    if let Some(handlers) = handler.get_mut(&ctrltype) {
        if !handlers.is_empty() {
            let handlers = std::mem::replace(handlers, Slab::new());
            for (_, handler) in handlers {
                handler.notify().ok();
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

fn register(ctrltype: u32, e: &Event) -> usize {
    let mut handler = HANDLER.lock().unwrap();
    let handle = e.handle();
    handler.entry(ctrltype).or_default().insert(handle)
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
struct CtrlEvent {
    ctrltype: u32,
    event: Event,
    handler_key: usize,
}

impl CtrlEvent {
    pub(crate) fn new(ctrltype: u32) -> io::Result<Self> {
        INIT.call_once(|| init().unwrap());

        let event = Event::new()?;
        let handler_key = register(ctrltype, &event);
        Ok(Self {
            ctrltype,
            event,
            handler_key,
        })
    }

    pub async fn wait(&self) -> io::Result<()> {
        self.event.wait().await
    }
}

impl Drop for CtrlEvent {
    fn drop(&mut self) {
        unregister(self.ctrltype, self.handler_key);
    }
}

async fn ctrl_event(ctrltype: u32) -> io::Result<()> {
    let event = CtrlEvent::new(ctrltype)?;
    event.wait().await
}

/// Creates a new listener which receives "ctrl-break" notifications sent to the
/// process.
pub async fn ctrl_break() -> io::Result<()> {
    ctrl_event(CTRL_BREAK_EVENT).await
}

/// Creates a new listener which receives "ctrl-close" notifications sent to the
/// process.
pub async fn ctrl_close() -> io::Result<()> {
    ctrl_event(CTRL_CLOSE_EVENT).await
}

/// Creates a new listener which receives "ctrl-c" notifications sent to the
/// process.
pub async fn ctrl_c() -> io::Result<()> {
    ctrl_event(CTRL_C_EVENT).await
}

/// Creates a new listener which receives "ctrl-logoff" notifications sent to
/// the process.
pub async fn ctrl_logoff() -> io::Result<()> {
    ctrl_event(CTRL_LOGOFF_EVENT).await
}

/// Creates a new listener which receives "ctrl-shutdown" notifications sent to
/// the process.
pub async fn ctrl_shutdown() -> io::Result<()> {
    ctrl_event(CTRL_SHUTDOWN_EVENT).await
}
