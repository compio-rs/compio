use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use slab::Slab;

use crate::runtime::Runtime;

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

    pub fn insert(&mut self, instant: Instant) -> Option<usize> {
        let delay = instant - self.time;
        if delay <= self.time.elapsed() {
            return None;
        }
        let key = self.tasks.insert(FutureState::Active(None));
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
}

impl TimerFuture {
    pub fn new(key: usize) -> Self {
        Self { key }
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Runtime::with_current(|r| r.poll_timer(cx, self.key))
    }
}

impl Drop for TimerFuture {
    fn drop(&mut self) {
        Runtime::with_current(|r| r.cancel_timer(self.key));
    }
}

#[test]
fn timer_min_timeout() {
    let mut runtime = TimerRuntime::new();
    assert_eq!(runtime.min_timeout(), None);

    let now = Instant::now();
    runtime.insert(now + Duration::from_secs(1));
    runtime.insert(now + Duration::from_secs(10));
    let min_timeout = runtime.min_timeout().unwrap().as_secs_f32();

    assert!(min_timeout < 1.);
}
