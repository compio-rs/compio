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

pub(crate) enum FutureState {
    Active(Option<Waker>),
    Completed,
}

impl Default for FutureState {
    fn default() -> Self {
        Self::Active(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimerKey {
    deadline: Instant,
    key: u64,
    _local_marker: PhantomData<*const ()>,
}

pub struct TimerRuntime {
    key: u64,
    wheel: BTreeMap<TimerKey, FutureState>,
}

impl TimerRuntime {
    pub fn new() -> Self {
        Self {
            key: 0,
            wheel: BTreeMap::default(),
        }
    }

    /// If the timer is completed, remove it and return `true`. Otherwise return
    /// `false` and keep it.
    pub fn remove_completed(&mut self, key: &TimerKey) -> bool {
        let completed = self
            .wheel
            .get(key)
            .map(|state| matches!(state, FutureState::Completed))
            .unwrap_or_default();
        if completed {
            self.wheel.remove(key);
        }
        completed
    }

    /// Insert a new timer. If the deadline is in the past, return `None`.
    pub fn insert(&mut self, deadline: Instant) -> Option<TimerKey> {
        if deadline <= Instant::now() {
            return None;
        }
        let key = TimerKey {
            key: self.key,
            deadline,
            _local_marker: PhantomData,
        };
        self.wheel.insert(key, FutureState::default());

        self.key += 1;

        Some(key)
    }

    /// Update the waker for a timer.
    pub fn update_waker(&mut self, key: &TimerKey, waker: Waker) {
        if let Some(w) = self.wheel.get_mut(key) {
            *w = FutureState::Active(Some(waker));
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

        self.wheel
            .iter_mut()
            .take_while(|(k, _)| k.deadline <= now)
            .for_each(|(_, v)| {
                if let FutureState::Active(Some(waker)) = replace(v, FutureState::Completed) {
                    waker.wake();
                }
            });
    }

    pub fn clear(&mut self) {
        self.wheel.clear();
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
