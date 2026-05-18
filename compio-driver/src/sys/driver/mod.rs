use std::sync::atomic::{AtomicU8, Ordering};

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
