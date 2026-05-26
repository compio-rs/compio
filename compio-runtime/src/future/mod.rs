use std::task::Waker;

use compio_buf::BufResult;
use compio_driver::{Extra, Key, OpCode, Proactor, PushEntry};
use compio_log::instrument;

fn poll_task<T: OpCode>(
    driver: &mut Proactor,
    waker: &Waker,
    key: Key<T>,
) -> PushEntry<Key<T>, BufResult<usize, T>> {
    instrument!(compio_log::Level::DEBUG, "poll_task", ?key);
    driver.pop(key).map_pending(|k| {
        driver.update_waker(&k, waker);
        k
    })
}

fn poll_task_with_extra<T: OpCode>(
    driver: &mut Proactor,
    waker: &Waker,
    key: Key<T>,
) -> PushEntry<Key<T>, (BufResult<usize, T>, Extra)> {
    instrument!(compio_log::Level::DEBUG, "poll_task_with_extra", ?key);
    driver.pop_with_extra(key).map_pending(|k| {
        driver.update_waker(&k, waker);
        k
    })
}

fn poll_multishot<T: OpCode>(
    driver: &mut Proactor,
    waker: &Waker,
    key: &Key<T>,
) -> Option<BufResult<usize, Extra>> {
    instrument!(compio_log::Level::DEBUG, "poll_multishot", ?key);
    if let Some(res) = driver.pop_multishot(key) {
        return Some(res);
    }
    driver.update_waker(key, waker);
    None
}

fn submit_raw<T: OpCode + 'static>(
    driver: &mut Proactor,
    op: T,
    extra: Option<Extra>,
) -> PushEntry<Key<T>, BufResult<usize, T>> {
    match extra {
        Some(e) => driver.push_with_extra(op, e),
        None => driver.push(op),
    }
}

mod combinator;
#[allow(clippy::module_inception)]
mod future;
mod stream;

pub use combinator::*;
pub use future::*;
pub use stream::*;
