//! Per-op cost of pushing and reaping io_uring NOPs through `Proactor`.
//!
//! A NOP has no fd (no `fget`/`fput`, no fsnotify) and completes inline, so
//! what's measured is compio's own submission and completion path.

#[cfg(io_uring)]
mod imp {
    use compio_buf::BufResult;
    use compio_driver::{IourOpCode, Key, OpEntry, Proactor, PushEntry};
    use criterion::{Criterion, Throughput};
    use io_uring::opcode;

    /// Ops pushed per wait, amortizing the `io_uring_enter` syscalls.
    const BATCH: usize = 64;

    /// Carries a payload so the op is about the size of a small `ReadAt`.
    struct Nop {
        _payload: [u8; 64],
    }

    unsafe impl IourOpCode for Nop {
        type Control = ();

        fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
            opcode::Nop::new().build().into()
        }
    }

    #[cfg(fusion)]
    unsafe impl compio_driver::PollOpCode for Nop {
        type Control = ();

        fn pre_submit(
            &mut self,
            _: &mut Self::Control,
        ) -> std::io::Result<compio_driver::Decision> {
            Err(std::io::ErrorKind::Unsupported.into())
        }

        fn operate(&mut self, _: &mut Self::Control) -> std::task::Poll<std::io::Result<usize>> {
            unreachable!("pre_submit never submits")
        }
    }

    fn run_batch(driver: &mut Proactor, keys: &mut Vec<Key<Nop>>, spare: &mut Vec<Key<Nop>>) {
        for _ in 0..BATCH {
            match driver.push(Nop { _payload: [0; 64] }) {
                PushEntry::Pending(k) => keys.push(k),
                PushEntry::Ready(BufResult(res, _)) => assert_eq!(res.unwrap(), 0),
            }
        }
        while !keys.is_empty() {
            driver.poll(None).unwrap();
            for k in keys.drain(..) {
                match driver.pop(k) {
                    PushEntry::Pending(k) => spare.push(k),
                    PushEntry::Ready(BufResult(res, _)) => assert_eq!(res.unwrap(), 0),
                }
            }
            std::mem::swap(keys, spare);
        }
    }

    pub fn submit(c: &mut Criterion) {
        let Ok(mut driver) = Proactor::new() else {
            eprintln!("io_uring unavailable, skipping");
            return;
        };
        if !driver.driver_type().is_iouring() {
            eprintln!("not io_uring, skipping");
            return;
        }
        let mut keys = Vec::with_capacity(BATCH);
        let mut spare = Vec::with_capacity(BATCH);
        let mut g = c.benchmark_group("submit");
        g.throughput(Throughput::Elements(BATCH as u64));
        g.bench_function("nop", |b| {
            b.iter(|| run_batch(&mut driver, &mut keys, &mut spare))
        });
        g.finish();
    }
}

#[cfg(io_uring)]
criterion::criterion_group!(benches, imp::submit);
#[cfg(io_uring)]
criterion::criterion_main!(benches);

#[cfg(not(io_uring))]
fn main() {}
