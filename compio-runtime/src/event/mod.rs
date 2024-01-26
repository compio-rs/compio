//! Asynchronous events.

use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use futures_util::{task::AtomicWaker, Future};

#[derive(Debug)]
struct Inner {
    waker: AtomicWaker,
    set: AtomicBool,
}

#[derive(Debug, Clone)]
struct Flag(Arc<Inner>);

impl Flag {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            waker: AtomicWaker::new(),
            set: AtomicBool::new(false),
        }))
    }

    pub fn signal(&self) {
        self.0.set.store(true, Ordering::Relaxed);
        self.0.waker.wake();
    }
}

impl Future for Flag {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // quick check to avoid registration if already done.
        if self.0.set.load(Ordering::Relaxed) {
            return Poll::Ready(());
        }

        self.0.waker.register(cx.waker());

        // Need to check condition **after** `register` to avoid a race
        // condition that would result in lost notifications.
        if self.0.set.load(Ordering::Relaxed) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// An event that won't wake until [`EventHandle::notify`] is called
/// successfully.
#[derive(Debug)]
pub struct Event {
    flag: Flag,
}

impl Default for Event {
    fn default() -> Self {
        Self::new()
    }
}

impl Event {
    /// Create [`Event`].
    pub fn new() -> Self {
        Self { flag: Flag::new() }
    }

    /// Get a notify handle.
    pub fn handle(&self) -> EventHandle {
        EventHandle::new(self.flag.clone())
    }

    /// Wait for [`EventHandle::notify`] called.
    pub async fn wait(self) {
        self.flag.await
    }
}

/// A wake up handle to [`Event`].
pub struct EventHandle {
    flag: Flag,
}

impl EventHandle {
    fn new(flag: Flag) -> Self {
        Self { flag }
    }

    /// Notify the event.
    pub fn notify(self) {
        self.flag.signal()
    }
}
