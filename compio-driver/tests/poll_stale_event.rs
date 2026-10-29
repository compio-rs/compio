//! Regression test for the poll driver dropping the rest of an event batch
//! when it receives a stale event.
//!
//! With a read and a write pending on the same fd, the driver registers both
//! interests under the key of the front write op. kqueue reports them as two
//! separate events carrying that key. If the writable event is handled first,
//! the write op completes, and the readable event then arrives with a key whose
//! op has no fd left to wait on. That stale event used to abort processing of
//! the whole batch, losing the (one-shot) events behind it, so their ops never
//! completed.

#![cfg(all(unix, any(not(target_os = "linux"), feature = "polling")))]

use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

use compio_driver::{
    DriverType, Proactor, PushEntry, SharedFd,
    op::{Interest, PollOnce},
};

#[test]
fn stale_event_does_not_drop_batch() {
    const PAIRS: usize = 32;

    let mut driver = Proactor::builder()
        .driver_type(DriverType::Poll)
        .build()
        .unwrap();

    let mut peers = Vec::with_capacity(PAIRS);
    let mut keys = Vec::with_capacity(PAIRS * 2);
    // kqueue reports ready knotes in the order they became active. Registering
    // while `a` is only writable, then making it readable, yields the batch
    // `w0, r0, w1, r1, ...`, where each `r` carries the completed write op's
    // key. A stale `r0` must not drop `w1` and onwards.
    for _ in 0..PAIRS {
        let (a, mut b) = UnixStream::pair().unwrap();
        let a = SharedFd::new(a);
        for interest in [Interest::Readable, Interest::Writable] {
            match driver.push(PollOnce::new(a.clone(), interest)) {
                PushEntry::Pending(key) => keys.push(Some(key)),
                PushEntry::Ready(_) => unreachable!("PollOnce always waits"),
            }
        }
        b.write_all(&[0]).unwrap();
        peers.push(b);
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while keys.iter().any(Option::is_some) {
        assert!(
            Instant::now() < deadline,
            "{} of {} ops stalled",
            keys.iter().filter(|k| k.is_some()).count(),
            PAIRS * 2,
        );
        match driver.poll(Some(Duration::from_millis(100))) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::ETIMEDOUT) => {}
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => panic!("poll failed: {e}"),
        }
        for slot in &mut keys {
            if let Some(key) = slot.take() {
                match driver.pop(key) {
                    PushEntry::Pending(key) => *slot = Some(key),
                    PushEntry::Ready(res) => {
                        res.0.unwrap();
                    }
                }
            }
        }
    }
}
