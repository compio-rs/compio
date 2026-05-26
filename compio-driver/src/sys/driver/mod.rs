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

/// An operation that can be optimized by making use of the "poll-first"
/// feature.
///
/// By setting this, `io_uring` will assume the socket is currently empty and
/// attempting to receive data will be unsuccessful. For this case, `io_uring`
/// will arm internal poll and trigger a receive of the data when the socket has
/// data to be read. This initial receive attempt can be wasteful for the case
/// where the socket is expected to be empty, setting this flag will bypass the
/// initial receive attempt and go straight to arming poll. If poll does
/// indicate that data is ready to be received, the operation will proceed.
pub trait PollFirst {
    /// Poll first before syscall. This is only meaningful for io-uring. It sets
    /// `IORING_RECVSEND_POLL_FIRST` flag in the `ioprio` of the SQE.
    fn poll_first(&mut self);
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
