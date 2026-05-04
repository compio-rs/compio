use std::sync::atomic::Ordering;

use compio_driver::{Extra, PollFirst};
#[cfg(feature = "sync")]
use synchrony::sync::atomic::AtomicU8;
#[cfg(not(feature = "sync"))]
use synchrony::unsync::atomic::AtomicU8;

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
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |state| {
                Some((state & !(0b11 << offset)) | bits)
            })
            .ok();
    }

    fn set_op(&self, offset: usize, op: &mut impl PollFirst) {
        if self.get_bit(offset) == Some(false) {
            op.poll_first();
        }
    }

    pub(super) fn set_recv(&self, extra: &Extra) {
        if let Ok(n) = extra.sock_nonempty() {
            self.set_bit(RECV_OFFSET, n);
        }
    }

    pub(super) fn set_recv_op(&self, op: &mut impl PollFirst) {
        self.set_op(RECV_OFFSET, op);
    }

    pub(super) fn set_accept(&self, extra: &Extra) {
        if let Ok(n) = extra.sock_nonempty() {
            self.set_bit(ACCEPT_OFFSET, n);
        }
    }

    pub(super) fn set_accept_op(&self, op: &mut impl PollFirst) {
        self.set_op(ACCEPT_OFFSET, op);
    }
}

impl Clone for SocketState {
    fn clone(&self) -> Self {
        let current = self.state.load(Ordering::Relaxed);
        Self {
            state: AtomicU8::new(current),
        }
    }
}
