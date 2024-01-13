//! Windows-specific types for signal handling.

#[cfg(feature = "lazy_cell")]
use std::sync::LazyLock;
use std::{
    collections::HashMap,
    io,
    sync::{Mutex, Once},
};

use compio_driver::syscall;
use futures_channel::oneshot::{channel, Receiver, Sender};
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

static HANDLER: LazyLock<Mutex<HashMap<u32, Slab<Sender<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

unsafe extern "system" fn ctrl_event_handler(ctrltype: u32) -> BOOL {
    let mut handler = HANDLER.lock().unwrap();
    if let Some(handlers) = handler.get_mut(&ctrltype) {
        if !handlers.is_empty() {
            let handlers = std::mem::replace(handlers, Slab::new());
            for (_, handler) in handlers {
                handler.send(()).ok();
            }
            return 1;
        }
    }
    0
}

static INIT: Once = Once::new();

fn init() -> io::Result<()> {
    syscall!(BOOL, SetConsoleCtrlHandler(Some(ctrl_event_handler), 1))?;
    Ok(())
}

fn register(ctrltype: u32, sender: Sender<()>) -> io::Result<usize> {
    let mut handler = HANDLER.lock().unwrap();
    Ok(handler.entry(ctrltype).or_default().insert(sender))
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
    receiver: Option<Receiver<()>>,
    handler_key: usize,
}

impl CtrlEvent {
    pub(crate) fn new(ctrltype: u32) -> io::Result<Self> {
        INIT.call_once(|| init().unwrap());

        let (sender, receiver) = channel();
        let handler_key = register(ctrltype, sender)?;
        Ok(Self {
            ctrltype,
            receiver: Some(receiver),
            handler_key,
        })
    }

    pub async fn wait(mut self) -> io::Result<()> {
        self.receiver
            .take()
            .expect("event could not be None")
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
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
