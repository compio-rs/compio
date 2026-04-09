#![cfg(loom)]

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use compio_executor::Executor;
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
