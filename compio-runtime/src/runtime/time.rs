use std::{
    collections::BinaryHeap,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use slab::Slab;

use crate::runtime::{FutureState, Runtime};

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
        other.delay.cmp(&self.delay)
    }
}

pub struct TimerRuntime {
    time: Instant,
    tasks: Slab<FutureState>,
    wheel: BinaryHeap<TimerEntry>,
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
        self.wheel.push(entry);
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
        let elapsed = self.time.elapsed();
        self.wheel.peek().map(|entry| {
            if entry.delay > elapsed {
                entry.delay - elapsed
            } else {
                Duration::ZERO
            }
        })
    }

    pub fn wake(&mut self) {
        let elapsed = self.time.elapsed();
        while let Some(entry) = self.wheel.pop() {
            if entry.delay <= elapsed {
                if let Some(state) = self.tasks.get_mut(entry.key) {
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
        Runtime::current().inner().poll_timer(cx, self.key)
    }
}

impl Drop for TimerFuture {
    fn drop(&mut self) {
        Runtime::current().inner().cancel_timer(self.key);
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
