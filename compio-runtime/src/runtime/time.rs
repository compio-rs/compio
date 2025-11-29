use std::{
    collections::BTreeMap,
    future::Future,
    marker::PhantomData,
    mem::replace,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use crate::runtime::Runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerKey {
    deadline: Instant,
    key: u64,
    _local_marker: PhantomData<*const ()>,
}

pub struct TimerRuntime {
    key: u64,
    wheel: BTreeMap<TimerKey, Waker>,
}

impl TimerRuntime {
    pub fn new() -> Self {
        Self {
            key: 0,
            wheel: BTreeMap::default(),
        }
    }

    /// Return true if the timer has completed.
    pub fn is_completed(&self, key: &TimerKey) -> bool {
        !self.wheel.contains_key(key)
    }

    /// Insert a new timer. If the deadline is in the past, return `None`.
    pub fn insert(&mut self, deadline: Instant) -> Option<TimerKey> {
        if deadline <= Instant::now() {
            return None;
        }
        let key = TimerKey {
            deadline,
            key: self.key,
            _local_marker: PhantomData,
        };
        self.wheel.insert(key, Waker::noop().clone());

        self.key += 1;

        Some(key)
    }

    /// Update the waker for a timer.
    pub fn update_waker(&mut self, key: &TimerKey, waker: Waker) {
        if let Some(w) = self.wheel.get_mut(key) {
            *w = waker;
        }
    }

    /// Cancel a timer.
    pub fn cancel(&mut self, key: &TimerKey) {
        self.wheel.remove(key);
    }

    /// Get the minimum timeout duration for the next poll.
    pub fn min_timeout(&self) -> Option<Duration> {
        self.wheel.first_key_value().map(|(key, _)| {
            let now = Instant::now();
            key.deadline.saturating_duration_since(now)
        })
    }

    /// Wake all the timer futures that have reached their deadline.
    pub fn wake(&mut self) {
        if self.wheel.is_empty() {
            return;
        }

        let now = Instant::now();

        let pending = self.wheel.split_off(&TimerKey {
            deadline: now,
            key: u64::MAX,
            _local_marker: PhantomData,
        });

        let expired = replace(&mut self.wheel, pending);
        for (_, w) in expired {
            w.wake();
        }
    }
}

pub struct TimerFuture(TimerKey);

impl TimerFuture {
    pub fn new(key: TimerKey) -> Self {
        Self(key)
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Runtime::with_current(|r| r.poll_timer(cx, &self.0))
    }
}

impl Drop for TimerFuture {
    fn drop(&mut self) {
        Runtime::with_current(|r| r.cancel_timer(&self.0));
    }
}

crate::assert_not_impl!(TimerFuture, Send, Sync);

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
