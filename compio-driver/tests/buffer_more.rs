//! `IORING_CQE_F_BUF_MORE` reaches the op through `Extra`.

#![cfg(io_uring)]

use std::{io::Write, num::NonZeroU16, os::unix::net::UnixStream};

use compio_driver::{
    AsRawFd, OpCode, Proactor, ProactorBuilder, PushEntry, SharedFd, op::RecvManaged,
};
use io_uring::types::IOU_PBUF_RING_INC;
use rustix::net::RecvFlags;

fn push_and_wait_with_extra<O: OpCode + 'static>(
    driver: &mut Proactor,
    op: O,
) -> (compio_buf::BufResult<usize, O>, compio_driver::Extra) {
    match driver.push(op) {
        PushEntry::Ready(res) => (res, driver.default_extra()),
        PushEntry::Pending(mut key) => loop {
            driver.poll(None).unwrap();
            match driver.pop_with_extra(key) {
                PushEntry::Pending(k) => key = k,
                PushEntry::Ready(res) => break res,
            }
        },
    }
}

/// Receive `data` into a single 4 KiB buffer of a pool set up with `flags`.
fn recv_buffer_more(flags: u16, data: &[u8]) -> Option<bool> {
    let mut driver = ProactorBuilder::new()
        .buffer_pool_size(NonZeroU16::new(1).unwrap())
        .buffer_pool_buffer_len(4096)
        .buffer_pool_flag(flags)
        .build()
        .unwrap();
    if !driver.driver_type().is_iouring() {
        eprintln!("the driver is not io_uring, skipping test");
        return None;
    }
    let pool = match driver.buffer_pool() {
        Ok(pool) => pool,
        Err(e) => {
            // `IOU_PBUF_RING_INC` needs Linux 6.12.
            eprintln!("cannot register the buffer ring with flags {flags:#x} ({e}), skipping test");
            return None;
        }
    };

    let (rx, mut tx) = UnixStream::pair().unwrap();
    let rx = SharedFd::new(rx);
    driver.attach(rx.as_raw_fd()).unwrap();
    tx.write_all(data).unwrap();

    let op = RecvManaged::new(rx, &pool, 0, RecvFlags::empty()).unwrap();
    let (res, extra) = push_and_wait_with_extra(&mut driver, op);
    assert_eq!(res.0.unwrap(), data.len());
    assert_eq!(extra.buffer_id().unwrap(), 0);
    Some(extra.buffer_more().unwrap())
}

#[test]
fn buffer_more_incremental() {
    // A short receive leaves most of the buffer to the kernel.
    if let Some(more) = recv_buffer_more(IOU_PBUF_RING_INC as _, b"hello") {
        assert!(more);
    }
}

#[test]
fn buffer_more_whole() {
    if let Some(more) = recv_buffer_more(0, b"hello") {
        assert!(!more);
    }
}
