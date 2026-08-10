use std::{
    cell::RefCell,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use pin_project_lite::pin_project;

use crate::{
    Runtime,
    time::{Elapsed, TimerRuntime, runtime::TimerKey, sleep_until},
};

#[derive(Debug)]
pub(crate) struct TimerFuture {
    key: TimerKey,
    rt: Rc<RefCell<TimerRuntime>>,
}

impl TimerFuture {
    /// Try to create a new `TimerFuture` if the instant is in the future;
    /// otherwise, a `None` will be returned.
    ///
    /// # Panics
    ///
    /// Panic if not running under a `Runtime`.
    pub fn try_new(instant: Instant) -> Option<Self> {
        Runtime::with_current(|rt| {
            let key = rt.timer_runtime.borrow_mut().insert(instant)?;
            Some(Self {
                key,
                rt: rt.timer_runtime.clone(),
            })
        })
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.rt.borrow_mut().poll_timer(cx, &self.key)
    }
}

impl Drop for TimerFuture {
    fn drop(&mut self) {
        self.rt.borrow_mut().cancel(&self.key)
    }
}

compio_driver::assert_not_impl!(TimerFuture, Send);
compio_driver::assert_not_impl!(TimerFuture, Sync);

/// Future returned by [`sleep`](super::sleep) and
/// [`sleep_until`](super::sleep_until).
#[must_use = "Futures do nothing unless polled."]
#[derive(Debug)]
pub struct Sleep(Option<TimerFuture>);

impl Sleep {
    #[inline]
    pub(crate) fn new(instant: Instant) -> Self {
        Sleep(TimerFuture::try_new(instant))
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(timer) = self.0.as_mut() {
            Pin::new(timer).poll(cx)
        } else {
            Poll::Ready(())
        }
    }
}

pin_project! {
    /// Future returned by [`timeout`](timeout) and [`timeout_at`](timeout_at).
    #[must_use = "Futures do nothing unless polled."]
    #[derive(Debug)]
    pub struct Timeout<F> {
        #[pin]
        fut: F,
        sleep: Sleep,
    }
}

impl<F: Future> Timeout<F> {
    pub(crate) fn new(instant: Instant, fut: F) -> Self {
        Self {
            fut,
            sleep: Sleep::new(instant),
        }
    }
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let me = self.project();

        if let Poll::Ready(out) = me.fut.poll(cx) {
            return Poll::Ready(Ok(out));
        }

        match Pin::new(me.sleep).poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(Elapsed(()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Interval returned by [`interval`] and [`interval_at`]
///
/// This type allows you to wait on a sequence of instants with a certain
/// duration between each instant. Unlike calling [`sleep`] in a loop, this lets
/// you count the time spent between the calls to [`sleep`] as well.
///
/// [`sleep`]: super::sleep
/// [`interval`]: super::interval
/// [`interval_at`]: super::interval_at
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
    /// See [`interval`](super::interval) and
    /// [`interval_at`](super::interval_at).
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
