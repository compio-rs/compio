//! Unix-specific types for signal handling.

use crate::{
    driver::{Driver, Poller},
    task::RUNTIME,
};
use once_cell::sync::Lazy as LazyLock;
use slab::Slab;
use std::{
    collections::HashMap,
    future::Future,
    io,
    pin::Pin,
    sync::{Mutex, Once},
    task::{Context, Poll},
};

#[allow(clippy::type_complexity)]
static HANDLER: LazyLock<Mutex<HashMap<i32, Slab<Box<dyn FnOnce() + Send + Sync>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

unsafe extern "C" fn signal_handler(sig: i32) {
    let mut handler = HANDLER.lock().unwrap();
    if let Some(handlers) = handler.get_mut(&sig) {
        if !handlers.is_empty() {
            let handlers = std::mem::replace(handlers, Slab::new());
            for (_, handler) in handlers {
                handler();
            }
        }
    }
}

static INIT: Once = Once::new();

fn init() {
    for sig in 0..libc::SIGRTMAX() {
        unsafe {
            libc::signal(sig, signal_handler as *const () as usize);
        }
    }
}

fn register(sig: i32, f: impl FnOnce() + Send + Sync + 'static) -> usize {
    let mut handler = HANDLER.lock().unwrap();
    handler.entry(sig).or_default().insert(Box::new(f))
}

fn unregister(sig: i32, key: usize) {
    let mut handler = HANDLER.lock().unwrap();
    if let Some(handlers) = handler.get_mut(&sig) {
        if handlers.contains(key) {
            let _ = handlers.remove(key);
        }
    }
}

/// Represents a listener to unix signal event.
#[derive(Debug)]
pub struct SignalEvent {
    sig: i32,
    user_data: usize,
    handler_key: usize,
}

impl SignalEvent {
    pub(crate) fn new(sig: i32) -> Self {
        INIT.call_once(init);

        let user_data = RUNTIME.with(|runtime| runtime.submit_dummy());
        let handler_key = RUNTIME.with(|runtime| {
            // Safety: the runtime is thread-local static, and the driver is send & sync.
            let driver = unsafe {
                (runtime.driver() as *const Driver)
                    .as_ref()
                    .unwrap_unchecked()
            };
            register(sig, move || driver.post(user_data, 0).unwrap())
        });
        Self {
            sig,
            user_data,
            handler_key,
        }
    }
}

impl Future for SignalEvent {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        RUNTIME
            .with(|runtime| runtime.poll_dummy(cx, self.user_data))
            .map(|res| res.map(|_| ()))
    }
}

impl Drop for SignalEvent {
    fn drop(&mut self) {
        unregister(self.sig, self.handler_key);
    }
}

/// Creates a new listener which will receive notifications when the current
/// process receives the specified signal.
pub fn signal(sig: i32) -> SignalEvent {
    SignalEvent::new(sig)
}
