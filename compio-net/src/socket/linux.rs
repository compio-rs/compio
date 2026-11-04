use std::sync::atomic::Ordering;

use compio_driver::{AsSocketState, Extra};
#[cfg(feature = "sync")]
use synchrony::sync::atomic::AtomicU8;
#[cfg(not(feature = "sync"))]
use synchrony::unsync::atomic::AtomicU8;

use super::SocketWithState;

const RECV_OFFSET: usize = 0;
const ACCEPT_OFFSET: usize = 2;

const UNSET: u8 = 0;
const EMPTY: u8 = 1;
const NON_EMPTY: u8 = 2;

#[derive(Debug)]
pub(super) struct SocketState {
    state: AtomicU8,
}

impl Default for SocketState {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketState {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
        }
    }

    fn get_bit(&self, offset: usize) -> Option<bool> {
        let state = self.state.load(Ordering::Relaxed);
        match (state >> offset) & 0b11 {
            UNSET => None,
            EMPTY => Some(false),
            NON_EMPTY => Some(true),
            _ => unreachable!(),
        }
    }

    fn set_bit(&self, offset: usize, value: bool) {
        let bits = if value { NON_EMPTY } else { EMPTY } << offset;
        self.state
            .update(Ordering::Relaxed, Ordering::Relaxed, |state| {
                (state & !(0b11 << offset)) | bits
            });
    }

    pub(super) fn set_recv(&self, extra: &Extra) {
        if let Ok(n) = extra.sock_nonempty() {
            self.set_bit(RECV_OFFSET, n);
        }
    }

    /// Whether the socket is known to be empty, so that a receive should arm
    /// poll before its first attempt.
    pub(super) fn recv_poll_first(&self) -> bool {
        self.get_bit(RECV_OFFSET) == Some(false)
    }

    pub(super) fn set_accept(&self, extra: &Extra) {
        if let Ok(n) = extra.sock_nonempty() {
            self.set_bit(ACCEPT_OFFSET, n);
        }
    }

    /// Same as [`SocketState::recv_poll_first`], for accept.
    pub(super) fn accept_poll_first(&self) -> bool {
        self.get_bit(ACCEPT_OFFSET) == Some(false)
    }
}

impl AsSocketState for SocketWithState {
    fn recv_poll_first(&self) -> bool {
        self.state.recv_poll_first()
    }

    fn accept_poll_first(&self) -> bool {
        self.state.accept_poll_first()
    }
}
