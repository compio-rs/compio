//! Utilities for tracking time.

use std::{
    error::Error,
    fmt::Display,
    future::Future,
    time::{Duration, Instant},
};

use futures_util::{select, FutureExt};

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
    Runtime::with_current(|r| r.create_timer(duration)).await
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
    sleep(deadline - Instant::now()).await
}

/// Error returned by [`timeout`] or [`timeout_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

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
        _ = sleep(duration).fuse() => Err(Elapsed),
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
