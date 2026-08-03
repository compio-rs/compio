//! Utilities for tracking time.

use std::{
    error::Error,
    fmt::Display,
    future::Future,
    time::{Duration, Instant},
};

mod runtime;
pub(crate) use runtime::TimerRuntime;

mod future;
pub use future::{Interval, Sleep, Timeout};

/// Error returned by [`timeout`] or [`timeout_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed(());

impl Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline has elapsed")
    }
}

impl Error for Elapsed {}

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
///
/// # Panics
///
/// Panic if not running under a `Runtime`.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep::new(Instant::now() + duration)
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
///
/// # Panics
///
/// Panic if not running under a `Runtime`.
pub fn sleep_until(deadline: Instant) -> Sleep {
    Sleep::new(deadline)
}

/// Require a [`Future`] to complete before the specified duration has elapsed.
///
/// If the future completes before the duration has elapsed, then the completed
/// value is returned. Otherwise, an error is returned and the future is
/// cancelled.
///
/// # Panics
///
/// Panic if not running under a `Runtime`.
pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F> {
    Timeout::new(Instant::now() + duration, future)
}

/// Require a [`Future`] to complete before the specified instant in time.
///
/// If the future completes before the instant is reached, then the completed
/// value is returned. Otherwise, an error is returned.
///
/// # Panics
///
/// Panic if not running under a `Runtime`.
pub fn timeout_at<F: Future>(deadline: Instant, future: F) -> Timeout<F> {
    Timeout::new(deadline, future)
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
