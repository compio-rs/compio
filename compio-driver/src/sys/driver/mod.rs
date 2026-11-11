use std::sync::atomic::{AtomicU8, Ordering};
#[cfg(any(windows, io_uring))]
use std::{
    io,
    time::{Duration, Instant},
};

#[cfg(any(windows, io_uring))]
use compio_log::warn;

cfg_select! {
    windows => {
        mod iocp;
        pub use iocp::*;
    }
    fusion => {
        mod fusion;
        mod poll;
        mod iour;
        pub use fusion::*;
    }
    io_uring => {
        mod iour;
        pub use iour::*;
    }
    stub => {
        mod stub;
        pub use stub::*;
    }
    unix => {
        mod poll;
        pub use poll::*;
    }
    _ => {}
}

crate::assert_not_impl!(Driver, Send);
crate::assert_not_impl!(Driver, Sync);

/// A driver that cancels and releases its in-flight operations when dropped.
#[cfg(any(windows, io_uring))]
pub(crate) trait CancelOnDrop {
    /// Name of the backend, used in diagnostics.
    const KIND: &'static str;

    /// Reap the operations that have already completed without blocking, and
    /// return how many are still in flight.
    fn reap_pending(&mut self) -> usize;

    /// Ask the backend to cancel every operation that is still in flight.
    fn cancel_pending(&mut self);

    /// Wait up to `timeout` for more completions.
    fn wait_pending(&mut self, timeout: Duration) -> io::Result<()>;

    /// Cancel the in-flight operations and wait for them to be released.
    fn shutdown(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;

        // Reap whatever has already finished, so that a driver that is only
        // dropping stale completions does not cancel anything.
        if self.reap_pending() == 0 {
            return;
        }

        self.cancel_pending();

        loop {
            if self.reap_pending() == 0 {
                break;
            }

            let now = Instant::now();
            if now >= deadline {
                break;
            }

            match self.wait_pending(deadline - now) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    warn!(
                        "failed to wait for {} completions on drop: {e:?}",
                        Self::KIND
                    );
                    break;
                }
            }
        }

        // A completion may have raced with the deadline check.
        let pending = self.reap_pending();
        if pending > 0 {
            warn!(
                "leaking {pending} in-flight {} op(s) on driver drop; the driver did not finish \
                 cancelling them within {timeout:?}",
                Self::KIND
            );
        }
    }
}

const IDLE: u8 = 0b00;
const NOTIFIED: u8 = 0b01;
const AWAKE: u8 = 0b10;

#[derive(Debug)]
struct AwakeFlag(AtomicU8);

impl AwakeFlag {
    pub fn new() -> Self {
        Self(AtomicU8::new(IDLE))
    }

    /// Mark the driver as awake by overwriting the flag byte with `AWAKE`.
    /// This intentionally clears any previously set `NOTIFIED` flag.
    pub fn set(&self) {
        self.0.store(AWAKE, Ordering::Release);
    }

    /// Reset the flags. Returns true if it was notified.
    pub fn reset(&self) -> bool {
        (self.0.swap(IDLE, Ordering::AcqRel) & NOTIFIED) != 0
    }

    /// Set the notified flag. Returns true if the awake flag is set or the
    /// notified flag is set. If the awake flag is not set, the driver needs
    /// to be notified through a syscall.
    pub fn wake(&self) -> bool {
        self.0.fetch_or(NOTIFIED, Ordering::AcqRel) != 0
    }
}
