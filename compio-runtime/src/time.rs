//! Utilities for tracking time.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt::Display,
    future::Future,
    marker::PhantomData,
    mem::replace,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use futures_util::{FutureExt, select};

use crate::Runtime;

/// Waits until `duration` has elapsed.
///
/// Equivalent to [`sleep_until(Instant::now() + duration)`](sleep_until). An
/// asynchronous analog to [`std::thread::sleep`].
///
/// To run something regularly on a schedule, see [`interval`].
///
/// # Examples
///
/// Wait 100ms and print "100 ms have elapsed".
///
/// ```
/// use std::time::Duration;
///
/// use compio_runtime::time::sleep;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// sleep(Duration::from_millis(100)).await;
/// println!("100 ms have elapsed");
/// # })
/// ```
pub async fn sleep(duration: Duration) {
    sleep_until(Instant::now() + duration).await
}

/// Waits until `deadline` is reached.
///
/// To run something regularly on a schedule, see [`interval`].
///
/// # Examples
///
/// Wait 100ms and print "100 ms have elapsed".
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use compio_runtime::time::sleep_until;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// sleep_until(Instant::now() + Duration::from_millis(100)).await;
/// println!("100 ms have elapsed");
/// # })
/// ```
pub async fn sleep_until(deadline: Instant) {
    crate::create_timer(deadline).await
}

/// Error returned by [`timeout`] or [`timeout_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed(());

impl Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline has elapsed")
    }
}

impl Error for Elapsed {}

/// Require a [`Future`] to complete before the specified duration has elapsed.
///
/// If the future completes before the duration has elapsed, then the completed
/// value is returned. Otherwise, an error is returned and the future is
/// canceled.
pub async fn timeout<F: Future>(duration: Duration, future: F) -> Result<F::Output, Elapsed> {
    select! {
        res = future.fuse() => Ok(res),
        _ = sleep(duration).fuse() => Err(Elapsed(())),
    }
}

/// Require a [`Future`] to complete before the specified instant in time.
///
/// If the future completes before the instant is reached, then the completed
/// value is returned. Otherwise, an error is returned.
pub async fn timeout_at<F: Future>(deadline: Instant, future: F) -> Result<F::Output, Elapsed> {
    timeout(deadline - Instant::now(), future).await
}

/// Interval returned by [`interval`] and [`interval_at`]
///
/// This type allows you to wait on a sequence of instants with a certain
/// duration between each instant. Unlike calling [`sleep`] in a loop, this lets
/// you count the time spent between the calls to [`sleep`] as well.
#[derive(Debug)]
pub struct Interval {
    first_ticked: bool,
    start: Instant,
    period: Duration,
}

impl Interval {
    pub(crate) fn new(start: Instant, period: Duration) -> Self {
        Self {
            first_ticked: false,
            start,
            period,
        }
    }

    /// Completes when the next instant in the interval has been reached.
    ///
    /// See [`interval`] and [`interval_at`].
    pub async fn tick(&mut self) -> Instant {
        if !self.first_ticked {
            sleep_until(self.start).await;
            self.first_ticked = true;
            self.start
        } else {
            let now = Instant::now();
            let next = now + self.period
                - Duration::from_nanos(
                    ((now - self.start).as_nanos() % self.period.as_nanos()) as _,
                );
            sleep_until(next).await;
            next
        }
    }
}

/// Creates new [`Interval`] that yields with interval of `period`. The first
/// tick completes immediately.
///
/// An interval will tick indefinitely. At any time, the [`Interval`] value can
/// be dropped. This cancels the interval.
///
/// This function is equivalent to
/// [`interval_at(Instant::now(), period)`](interval_at).
///
/// # Panics
///
/// This function panics if `period` is zero.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use compio_runtime::time::interval;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// let mut interval = interval(Duration::from_millis(10));
///
/// interval.tick().await; // ticks immediately
/// interval.tick().await; // ticks after 10ms
/// interval.tick().await; // ticks after 10ms
///
/// // approximately 20ms have elapsed.
/// # })
/// ```
///
/// A simple example using [`interval`] to execute a task every two seconds.
///
/// The difference between [`interval`] and [`sleep`] is that an [`Interval`]
/// measures the time since the last tick, which means that [`.tick().await`]
/// may wait for a shorter time than the duration specified for the interval
/// if some time has passed between calls to [`.tick().await`].
///
/// If the tick in the example below was replaced with [`sleep`], the task
/// would only be executed once every three seconds, and not every two
/// seconds.
///
/// ```no_run
/// use std::time::Duration;
///
/// use compio_runtime::time::{interval, sleep};
///
/// async fn task_that_takes_a_second() {
///     println!("hello");
///     sleep(Duration::from_secs(1)).await
/// }
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// let mut interval = interval(Duration::from_secs(2));
/// for _i in 0..5 {
///     interval.tick().await;
///     task_that_takes_a_second().await;
/// }
/// # })
/// ```
///
/// [`sleep`]: crate::time::sleep()
/// [`.tick().await`]: Interval::tick
pub fn interval(period: Duration) -> Interval {
    interval_at(Instant::now(), period)
}

/// Creates new [`Interval`] that yields with interval of `period` with the
/// first tick completing at `start`.
///
/// An interval will tick indefinitely. At any time, the [`Interval`] value can
/// be dropped. This cancels the interval.
///
/// # Panics
///
/// This function panics if `period` is zero.
///
/// # Examples
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use compio_runtime::time::interval_at;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// let start = Instant::now() + Duration::from_millis(50);
/// let mut interval = interval_at(start, Duration::from_millis(10));
///
/// interval.tick().await; // ticks after 50ms
/// interval.tick().await; // ticks after 10ms
/// interval.tick().await; // ticks after 10ms
///
/// // approximately 70ms have elapsed.
/// # });
/// ```
pub fn interval_at(start: Instant, period: Duration) -> Interval {
    assert!(period > Duration::ZERO, "`period` must be non-zero.");
    Interval::new(start, period)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TimerKey {
    deadline: Instant,
    key: u64,
    _local_marker: PhantomData<*const ()>,
}

pub(crate) struct TimerRuntime {
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
    pub fn update_waker(&mut self, key: &TimerKey, waker: &Waker) {
        if let Some(w) = self.wheel.get_mut(key)
            && !waker.will_wake(w)
        {
            *w = waker.clone();
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

pub(crate) struct TimerFuture(TimerKey);

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

compio_driver::assert_not_impl!(TimerFuture, Send);
compio_driver::assert_not_impl!(TimerFuture, Sync);

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
