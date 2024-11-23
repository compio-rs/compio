use std::{
    cell::RefCell,
    cmp::Reverse,
    collections::BinaryHeap,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use compio_log::{debug, instrument};
use slab::Slab;

pub(crate) enum FutureState {
    Active(Option<Waker>),
    Completed,
}

impl Default for FutureState {
    fn default() -> Self {
        Self::Active(None)
    }
}

#[derive(Debug)]
struct TimerEntry {
    key: usize,
    delay: Duration,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.delay == other.delay
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.delay.cmp(&other.delay)
    }
}

pub struct TimerRuntime {
    time: Instant,
    tasks: Slab<FutureState>,
    wheel: BinaryHeap<Reverse<TimerEntry>>,
}

impl TimerRuntime {
    pub fn new() -> Self {
        Self {
            time: Instant::now(),
            tasks: Slab::default(),
            wheel: BinaryHeap::default(),
        }
    }

    pub fn is_completed(&self, key: usize) -> bool {
        self.tasks
            .get(key)
            .map(|state| matches!(state, FutureState::Completed))
            .unwrap_or_default()
    }

    pub fn insert(&mut self, mut delay: Duration) -> Option<usize> {
        if delay.is_zero() {
            return None;
        }
        let elapsed = self.time.elapsed();
        let key = self.tasks.insert(FutureState::Active(None));
        delay += elapsed;
        let entry = TimerEntry { key, delay };
        self.wheel.push(Reverse(entry));
        Some(key)
    }

    pub fn update_waker(&mut self, key: usize, waker: Waker) {
        if let Some(w) = self.tasks.get_mut(key) {
            *w = FutureState::Active(Some(waker));
        }
    }

    pub fn cancel(&mut self, key: usize) {
        self.tasks.remove(key);
    }

    pub fn min_timeout(&self) -> Option<Duration> {
        self.wheel.peek().map(|entry| {
            let elapsed = self.time.elapsed();
            if entry.0.delay > elapsed {
                entry.0.delay - elapsed
            } else {
                Duration::ZERO
            }
        })
    }

    pub fn wake(&mut self) {
        if self.wheel.is_empty() {
            return;
        }
        let elapsed = self.time.elapsed();
        while let Some(entry) = self.wheel.pop() {
            if entry.0.delay <= elapsed {
                if let Some(state) = self.tasks.get_mut(entry.0.key) {
                    let old_state = std::mem::replace(state, FutureState::Completed);
                    if let FutureState::Active(Some(waker)) = old_state {
                        waker.wake();
                    }
                }
            } else {
                self.wheel.push(entry);
                break;
            }
        }
    }
}

pub struct TimerFuture {
    key: usize,
    runtime: Rc<RefCell<TimerRuntime>>,
}

impl TimerFuture {
    pub fn new(key: usize, runtime: Rc<RefCell<TimerRuntime>>) -> Self {
        Self { key, runtime }
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        instrument!(compio_log::Level::DEBUG, "poll_timer", ?cx, ?self.key);
        let mut timer_runtime = self.runtime.borrow_mut();
        if !timer_runtime.is_completed(self.key) {
            debug!("pending");
            timer_runtime.update_waker(self.key, cx.waker().clone());
            Poll::Pending
        } else {
            debug!("ready");
            Poll::Ready(())
        }
    }
}

impl Drop for TimerFuture {
    fn drop(&mut self) {
        self.runtime.borrow_mut().cancel(self.key);
    }
}

#[test]
fn timer_min_timeout() {
    let mut runtime = TimerRuntime::new();
    assert_eq!(runtime.min_timeout(), None);

    runtime.insert(Duration::from_secs(1));
    runtime.insert(Duration::from_secs(10));
    let min_timeout = runtime.min_timeout().unwrap().as_secs_f32();

    assert!(min_timeout < 1.);
}
