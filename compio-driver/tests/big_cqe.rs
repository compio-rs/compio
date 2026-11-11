//! The second half of a 32-byte CQE reaches the op through `Extra`.

#![cfg(all(io_uring, not(fusion), feature = "io-uring-cqe32"))]

use std::io;

use compio_buf::BufResult;
use compio_driver::{Extra, OpCode, OpEntry, Proactor, PushEntry};
use io_uring::opcode;
use linux_raw_sys::io_uring::io_uring_sqe;

/// Post a 32-byte CQE carrying `sqe->off` and `sqe->addr` (Linux 6.18).
const IORING_NOP_CQE32: u32 = 1 << 5;

const PAYLOAD: [u64; 2] = [0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210];

struct NopCqe32 {
    big_cqe: Option<io::Result<[u64; 2]>>,
}

unsafe impl OpCode for NopCqe32 {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let mut entry = opcode::Nop::new().build();
        let sqe = &raw mut entry as *mut io_uring_sqe;
        // SAFETY: `Entry` is a `repr(C)` wrapper of `io_uring_sqe`.
        unsafe {
            (*sqe).__bindgen_anon_3.nop_flags = IORING_NOP_CQE32;
            (*sqe).__bindgen_anon_1.off = PAYLOAD[0];
            (*sqe).__bindgen_anon_2.addr = PAYLOAD[1];
        }
        entry.into()
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, _: &io::Result<usize>, extra: &Extra) {
        self.big_cqe = Some(extra.big_cqe());
    }
}

#[test]
fn big_cqe() {
    let mut driver = Proactor::new().unwrap();
    if !driver.driver_type().is_iouring() {
        eprintln!("the driver is not io_uring, skipping test");
        return;
    }

    let BufResult(res, op) = match driver.push(NopCqe32 { big_cqe: None }) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut key) => loop {
            driver.poll(None).unwrap();
            match driver.pop(key) {
                PushEntry::Pending(k) => key = k,
                PushEntry::Ready(res) => break res,
            }
        },
    };
    if let Err(e) = res {
        assert_eq!(e.raw_os_error(), Some(libc::EINVAL), "{e}");
        eprintln!("IORING_NOP_CQE32 needs Linux 6.18, skipping test");
        return;
    }
    assert_eq!(op.big_cqe.unwrap().unwrap(), PAYLOAD);
}
