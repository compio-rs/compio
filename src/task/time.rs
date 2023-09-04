use slab::Slab;
use std::{
    collections::{BinaryHeap, HashMap},
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

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
    tasks: Slab<()>,
    wheel: BinaryHeap<TimerEntry>,
    wakers: HashMap<usize, Option<Waker>>,
}

impl TimerRuntime {
    pub fn new() -> Self {
        Self {
            time: Instant::now(),
            tasks: Slab::default(),
            wheel: BinaryHeap::default(),
            wakers: HashMap::default(),
        }
    }

    pub fn contains(&self, key: usize) -> bool {
        self.wakers.contains_key(&key)
    }

    pub fn insert(&mut self, mut delay: Duration) -> Option<usize> {
        if delay.is_zero() {
            return None;
        }
        let elapsed = self.time.elapsed();
        let key = self.tasks.insert(());
        delay += elapsed;
        let entry = TimerEntry { key, delay };
        self.wheel.push(entry);
        self.wakers.insert(key, None);
        Some(key)
    }

    pub fn update_waker(&mut self, key: usize, waker: Waker) {
        self.wakers.insert(key, Some(waker));
    }

    pub fn cancel(&mut self, key: usize) {
        self.wakers.remove(&key);
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
                if let Some(waker) = self.wakers.remove(&entry.key).flatten() {
                    waker.wake();
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
    completed: bool,
}

impl TimerFuture {
    pub fn new(key: usize) -> Self {
        Self {
            key,
            completed: false,
        }
    }
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = crate::task::RUNTIME.with(|runtime| runtime.poll_timer(cx, self.key));
        if res.is_ready() {
            self.get_mut().completed = true;
        }
        res
    }
}

impl Drop for TimerFuture {
    fn drop(&mut self) {
        if !self.completed {
            crate::task::RUNTIME.with(|runtime| runtime.cancel_timer(self.key));
        }
    }
}
