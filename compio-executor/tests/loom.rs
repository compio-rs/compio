#![cfg(loom)]

use std::{
    future::{Future, ready},
    pin::Pin,
    task::{Context, Poll, Waker},
};

use compio_executor::Executor;
use compio_log::info;
use loom::thread;

fn block_on<F: Future + 'static>(exe: &Executor, f: F) -> F::Output {
    let cx = &mut Context::from_waker(Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(res) = f.as_mut().poll(cx) {
            return res;
        }
        exe.tick();
        thread::yield_now();
    }
}

struct Yield(bool);

impl Future for Yield {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

struct CrossThreadWake(bool);

impl Future for CrossThreadWake {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            let waker = cx.waker().clone();
            thread::spawn(move || waker.wake());
            Poll::Pending
        }
    }
}

/// Model of the `Remote::schedule` / executor scheduling state machine.
/// Used by the `test_no_lost_cross_thread_wake` loom test below.
///
/// Uses a `Mutex<VecDeque>` in place of `ArrayQueue` so every queue
/// operation is visible to loom (crossbeam's internals are not
/// loom-aware).
mod schedule_model {
    use std::collections::VecDeque;

    use loom::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering::*},
    };

    const SCHEDULED: usize = 1 << 0;
    const SCHEDULING: usize = 1 << 1;

    pub(super) struct Task {
        state: AtomicUsize,
        queue: Mutex<VecDeque<()>>,
    }

    impl Task {
        pub fn new() -> Self {
            Self {
                state: AtomicUsize::new(0),
                queue: Mutex::new(VecDeque::new()),
            }
        }

        /// Models `Remote::schedule` (fixed: no `is_scheduling()` guard).
        pub fn remote_schedule(&self) {
            let prev = self.state.fetch_or(SCHEDULED | SCHEDULING, AcqRel);
            if prev & SCHEDULED != 0 {
                self.state.fetch_and(!SCHEDULING, Release);
                return;
            }
            self.queue.lock().unwrap().push_back(());
            self.state.fetch_and(!SCHEDULING, Release);
        }

        /// Models executor: pop one entry, clear SCHEDULED.
        pub fn try_tick(&self) -> bool {
            let popped = self.queue.lock().unwrap().pop_front().is_some();
            if popped {
                self.state.fetch_and(!SCHEDULED, AcqRel);
            }
            popped
        }

        pub fn drain(&self) {
            while self.try_tick() {}
        }

        /// SCHEDULED=1 with an empty queue means the task is stranded.
        pub fn assert_no_stranded_task(&self) {
            let bits = self.state.load(Acquire);
            let empty = self.queue.lock().unwrap().is_empty();
            assert_eq!(bits & SCHEDULING, 0, "SCHEDULING still set after join");
            assert!(
                !(bits & SCHEDULED != 0 && empty),
                "lost wake: SCHEDULED=1 but queue empty — task stranded",
            );
        }
    }
}

async fn yield_now() {
    Yield(false).await
}

#[test]
fn test_spawn_and_run() {
    loom::model(|| {
        let exe = Executor::new();
        let handle = exe.spawn(async {
            yield_now().await;
            42usize
        });

        let res = block_on(&exe, handle);
        assert_eq!(res.unwrap(), 42);
    });
}

#[test]
fn test_concurrent_cancel_and_run() {
    loom::model(|| {
        let exe = Executor::new();

        let handle = exe.spawn(async {
            yield_now().await;
            42usize
        });

        let drop_thread = thread::spawn(move || {
            drop(handle);
        });

        // Keep ticking until there are no more tasks
        while exe.has_task() {
            exe.tick();
        }

        drop_thread.join().unwrap();
    });
}

#[test]
fn test_cross_thread_wake() {
    loom::model(|| {
        let exe = Executor::new();

        let handle = exe.spawn(CrossThreadWake(false));
        block_on(&exe, handle).unwrap();
    });
}

/// Regression test for <https://github.com/compio-rs/compio/issues/948>.
///
/// Models `Remote::schedule` with two concurrent cross-thread wakers and
/// an executor that can interleave. Loom exhaustively checks that no
/// interleaving strands the task (SCHEDULED=1 with an empty queue).
///
/// Uses a direct model of the scheduling state machine rather than the
/// full `Executor` to keep the state space tractable for loom (~48s).
#[test]
fn test_no_lost_cross_thread_wake() {
    use loom::sync::Arc;
    use schedule_model::Task;

    loom::model(|| {
        let task = Arc::new(Task::new());

        let t1 = task.clone();
        let w1 = thread::spawn(move || t1.remote_schedule());

        let te = task.clone();
        let exec = thread::spawn(move || {
            for _ in 0..2 {
                te.try_tick();
            }
        });

        let t2 = task.clone();
        let w2 = thread::spawn(move || t2.remote_schedule());

        w1.join().unwrap();
        w2.join().unwrap();
        exec.join().unwrap();

        task.drain();
        task.assert_no_stranded_task();
    });
}

#[test]
fn test_join_while_complete() {
    loom::model(|| {
        let exe = Executor::new();

        let handle = exe.spawn(ready(0));

        let thread1 = thread::spawn(move || {
            let cx = &mut Context::from_waker(Waker::noop());
            let mut f = std::pin::pin!(handle);
            loop {
                info!("Poll from thread 1");
                if let Poll::Ready(res) = f.as_mut().poll(cx) {
                    return res;
                }
                thread::yield_now();
            }
        });

        while exe.tick() {
            thread::yield_now();
        }

        let _ = thread1.join().unwrap();
    });
}
