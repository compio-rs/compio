use std::{
    collections::BTreeMap,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use compio_log::{debug, instrument};

use super::Clock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TimerKey {
    deadline: Instant,
    generation: u64,
}

pub(crate) struct TimerRuntime {
    generation: u64,
    wheel: BTreeMap<TimerKey, Option<Waker>>,
    clock: Option<Arc<dyn Clock>>,
}

impl std::fmt::Debug for TimerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimerRuntime")
            .field("generation", &self.generation)
            .field("wheel", &self.wheel)
            .field("clock", &self.clock.is_some())
            .finish()
    }
}

impl TimerRuntime {
    pub fn new(clock: Option<Arc<dyn Clock>>) -> Self {
        Self {
            generation: 0,
            wheel: BTreeMap::default(),
            clock,
        }
    }

    pub fn now(&self) -> Instant {
        match &self.clock {
            Some(clock) => clock.now(),
            None => Instant::now(),
        }
    }

    /// Return true if the timer has completed.
    pub fn is_completed(&self, key: &TimerKey) -> bool {
        !self.wheel.contains_key(key)
    }

    /// Insert a new timer. If the deadline is in the past, return `None`.
    pub fn insert(&mut self, deadline: Instant) -> Option<TimerKey> {
        if deadline <= self.now() {
            return None;
        }
        let key = TimerKey {
            deadline,
            generation: self.generation,
        };
        self.wheel.insert(key, None);

        self.generation = self
            .generation
            .checked_add(1)
            .expect("too many timers created");

        Some(key)
    }

    /// Update the waker for a timer.
    pub fn update_waker(&mut self, key: &TimerKey, waker: &Waker) {
        // Only set the waker if the timer is not completed
        let Some(w) = self.wheel.get_mut(key) else {
            return;
        };

        // If there's already a waker set, check for duplication
        if let Some(w) = w
            && waker.will_wake(w)
        {
            return;
        }

        *w = Some(waker.clone())
    }

    /// Cancel a timer.
    pub fn cancel(&mut self, key: &TimerKey) {
        self.wheel.remove(key);
    }

    /// Get the minimum timeout duration for the next poll.
    pub fn min_timeout(&self) -> Option<Duration> {
        self.wheel.first_key_value().map(|(key, _)| {
            let now = self.now();
            key.deadline.saturating_duration_since(now)
        })
    }

    /// Wake all the timer futures that have reached their deadline.
    pub fn wake(&mut self) {
        if self.wheel.is_empty() {
            return;
        }

        let now = self.now();
        let due = ..=TimerKey {
            deadline: now,
            generation: u64::MAX,
        };

        let expired = self.wheel.extract_if(due, |_, _| true);
        for (_, w) in expired {
            if let Some(w) = w {
                w.wake();
            }
        }
    }

    pub fn poll_timer(&mut self, cx: &mut Context<'_>, key: &TimerKey) -> Poll<()> {
        instrument!(compio_log::Level::DEBUG, "poll_timer", ?cx, ?key);
        if self.is_completed(key) {
            debug!("ready");
            Poll::Ready(())
        } else {
            debug!("pending");
            self.update_waker(key, cx.waker());
            Poll::Pending
        }
    }
}
