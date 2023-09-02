//! Utilities for tracking time.

use futures_util::{select, FutureExt};
use std::{
    error::Error,
    fmt::Display,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

pub async fn sleep(duration: Duration) {
    crate::task::RUNTIME
        .with(|runtime| runtime.create_timer(duration))
        .await
}

pub async fn sleep_until(deadline: Instant) {
    sleep(deadline - Instant::now()).await
}

#[derive(Debug, PartialEq, Eq)]
pub struct Elapsed;

impl Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("deadline has elapsed")
    }
}

impl Error for Elapsed {}

pub async fn timeout<F: Future>(duration: Duration, future: F) -> Result<F::Output, Elapsed> {
    select! {
        res = future.fuse() => Ok(res),
        _ = sleep(duration).fuse() => Err(Elapsed),
    }
}

pub async fn timeout_at<F: Future>(deadline: Instant, future: F) -> Result<F::Output, Elapsed> {
    timeout(deadline - Instant::now(), future).await
}

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

pub fn interval(period: Duration) -> Interval {
    interval_at(Instant::now(), period)
}

pub fn interval_at(start: Instant, period: Duration) -> Interval {
    Interval::new(start, period)
}
