//! A batch larger than the submission queue: the driver has to submit to make
//! room and push again, and every op still completes.
#![cfg(io_uring)]

use compio_buf::BufResult;
use compio_driver::{IourOpCode, OpEntry, Proactor, PushEntry};
use io_uring::opcode;

struct Nop;

unsafe impl IourOpCode for Nop {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Nop::new().build().into()
    }
}

#[cfg(fusion)]
unsafe impl compio_driver::PollOpCode for Nop {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> std::io::Result<compio_driver::Decision> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    fn operate(&mut self, _: &mut Self::Control) -> std::task::Poll<std::io::Result<usize>> {
        unreachable!("pre_submit never submits")
    }
}

#[test]
fn batch_larger_than_sq() {
    let Ok(mut driver) = Proactor::builder().capacity(4).build() else {
        eprintln!("io_uring unavailable, skipping");
        return;
    };
    if !driver.driver_type().is_iouring() {
        eprintln!("not io_uring, skipping");
        return;
    }
    let mut keys: Vec<_> = (0..64)
        .filter_map(|_| match driver.push(Nop) {
            PushEntry::Pending(k) => Some(k),
            PushEntry::Ready(BufResult(res, _)) => {
                assert_eq!(res.unwrap(), 0);
                None
            }
        })
        .collect();
    while !keys.is_empty() {
        driver.poll(None).unwrap();
        keys = keys
            .into_iter()
            .filter_map(|k| match driver.pop(k) {
                PushEntry::Pending(k) => Some(k),
                PushEntry::Ready(BufResult(res, _)) => {
                    assert_eq!(res.unwrap(), 0);
                    None
                }
            })
            .collect();
    }
}
