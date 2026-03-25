use std::{
    mem::{self},
    process::abort,
};

mod slot_queue;
pub use slot_queue::*;

mod one_shot;
pub use one_shot::*;

pub(crate) struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        abort();
    }
}

/// Calls a function and aborts if it panics.
///
/// This is useful in unsafe code where we can't recover from panics.
pub(crate) fn abort_on_panic<T>(f: impl FnOnce() -> T) -> T {
    let bomb = Bomb;
    let t = f();
    mem::forget(bomb);
    t
}
