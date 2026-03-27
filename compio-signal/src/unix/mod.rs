//! Unix-specific types for signal handling.

use std::{io, sync::LazyLock};

mod half_lock;

use nix::sys::signal::{self, SigHandler, Signal};
use slab::Slab;
use synchrony::sync::async_flag::{AsyncFlag as Event, AsyncFlagHandle as EventHandle};

use crate::unix::half_lock::HalfLock;

static HANDLER: LazyLock<HalfLock<Slab<(Signal, EventHandle)>>> = LazyLock::new(HalfLock::default);

extern "C" fn signal_handler(sig: i32) {
    let Ok(sig) = Signal::try_from(sig) else {
        return;
    };
    for handler in HANDLER
        .read()
        .iter()
        .filter_map(|(_, (s, handler))| (sig == *s).then_some(handler))
    {
        handler.clone().notify();
    }
}

fn register(sig: Signal, event: &Event) -> io::Result<usize> {
    let handle = event.handle();
    let mut guard = HANDLER.write();
    let mut new = Slab::clone(&*guard);
    let key = new.insert((sig, handle));
    guard.store(new);
    unsafe { signal::signal(sig, SigHandler::Handler(signal_handler)) }?;

    Ok(key)
}

fn unregister(sig: Signal, key: usize) -> io::Result<()> {
    let mut handler = HANDLER.write();
    let mut new = Slab::clone(&*handler);
    new.remove(key);
    let need_uninit = new.iter().all(|(_, (s, _))| *s != sig);

    if need_uninit {
        unsafe { signal::signal(sig, SigHandler::SigDfl) }?;
    }

    handler.store(new);

    Ok(())
}

/// A listener to unix signal event.
#[derive(Debug)]
struct SignalListener {
    sig: Signal,
    key: usize,
    event: Option<Event>,
}

impl SignalListener {
    fn new(sig: i32) -> io::Result<Self> {
        let sig = Signal::try_from(sig)?;
        let event = Event::new();
        let key = register(sig, &event)?;
        Ok(Self {
            sig,
            key,
            event: Some(event),
        })
    }

    async fn wait(mut self) {
        self.event
            .take()
            .expect("event could not be None")
            .wait()
            .await
    }
}

impl Drop for SignalListener {
    fn drop(&mut self) {
        _ = unregister(self.sig, self.key);
    }
}

/// Creates a new listener which will receive notifications when the current
/// process receives the specified signal.
pub async fn signal(sig: i32) -> io::Result<()> {
    let fd = SignalListener::new(sig)?;
    fd.wait().await;
    Ok(())
}
