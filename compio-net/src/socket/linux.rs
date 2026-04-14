use std::{ops::Deref, sync::atomic::Ordering};

#[cfg(feature = "sync")]
use synchrony::sync::atomic::AtomicI8;
#[cfg(not(feature = "sync"))]
use synchrony::unsync::atomic::AtomicI8;

// We are not on the IO_URING driver and hence retrieving socket state is
// not supported.
const UNSUPPORTED: i8 = -1;

// The socket was empty after the receive operation.
const EMPTY: i8 = 0;

// The socket was not-empty after the last receive operation and has more
// data to be read.
const NON_EMPTY: i8 = 1;

#[derive(Debug)]
pub(super) struct SocketState {
    state: AtomicI8,
}

impl Default for SocketState {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketState {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicI8::new(-1),
        }
    }

    pub(super) fn get(&self) -> Option<bool> {
        match self.load(Ordering::Relaxed) {
            UNSUPPORTED => None,
            EMPTY => Some(false),
            NON_EMPTY => Some(true),
            _ => unreachable!(),
        }
    }

    pub(super) fn set(&self, extra: &compio_driver::Extra) {
        if let Ok(n) = extra.sock_nonempty() {
            self.store(n as i8, Ordering::Relaxed);
        }
    }
}

impl Clone for SocketState {
    fn clone(&self) -> Self {
        let current = self.state.load(Ordering::Relaxed);
        Self {
            state: AtomicI8::new(current),
        }
    }
}

impl Deref for SocketState {
    type Target = AtomicI8;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
