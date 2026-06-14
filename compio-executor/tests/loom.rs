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
/// Exercises the real `Executor` / `Remote::schedule` path with two
/// concurrent cross-thread wakes. A message-pump task parks after each
/// wake; loom exhaustively checks that no interleaving strands the task
/// (W2's wake lost because W1 is still inside `finish_scheduling`).
#[test]
fn test_no_lost_cross_thread_wake() {
    use std::{
        cell::{Cell, RefCell},
        future::poll_fn,
        rc::Rc,
    };

    loom::model(|| {
        let exe = Executor::new();
        let polls = Rc::new(Cell::new(0));
        let waker = Rc::new(RefCell::new(None));

        let handle = exe.spawn({
            let (polls, waker) = (polls.clone(), waker.clone());
            poll_fn(move |cx| {
                let n = polls.get() + 1;
                polls.set(n);
                *waker.borrow_mut() = Some(cx.waker().clone());
                if n >= 3 {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
        });

        exe.tick();
        assert_eq!(polls.get(), 1, "spawned task did not run on first tick");

        let w1: Waker = waker.borrow().clone().unwrap();
        let t1 = thread::spawn(move || w1.wake());

        loop {
            exe.tick();
            if polls.get() >= 2 {
                break;
            }
            thread::yield_now();
        }

        let w2: Waker = waker.borrow().clone().unwrap();
        let t2 = thread::spawn(move || w2.wake());
        t2.join().unwrap();
        t1.join().unwrap();

        for _ in 0..4 {
            exe.tick();
        }

        assert_eq!(
            polls.get(),
            3,
            "lost wake: both wake() calls returned, yet the task was never re-polled (parked \
             forever with a message pending)",
        );

        drop(handle);
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
