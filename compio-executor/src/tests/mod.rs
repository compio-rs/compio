use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use super::*;

std::thread_local! {
    static EXE: Executor = Executor::new();
}

fn spawn<F: Future + 'static>(f: F) -> JoinHandle<F::Output> {
    EXE.with(|exe| exe.spawn(f))
}

fn block_on<F: Future + 'static>(f: F) -> F::Output {
    EXE.with(|exe| {
        let mut cx = Context::from_waker(Waker::noop());
        let mut fut = pin!(f);
        loop {
            if let Poll::Ready(res) = fut.as_mut().poll(&mut cx) {
                return res;
            }
            exe.tick();
        }
    })
}

struct Yield(bool);

impl Future for Yield {
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

async fn yield_now() {
    Yield(false).await
}

#[test]
fn test_executor_runs_to_completion() {
    block_on(async {
        let values = std::rc::Rc::new(RefCell::new(Vec::new()));

        let a = {
            let values = values.clone();
            spawn(async move {
                for i in 0..5 {
                    values.borrow_mut().push(("a", i));
                    yield_now().await;
                }
                10usize
            })
        };

        let b = {
            let values = values.clone();
            spawn(async move {
                for i in 0..3 {
                    values.borrow_mut().push(("b", i));
                    yield_now().await;
                }
                20usize
            })
        };

        let ra = a.await.unwrap();
        let rb = b.await.unwrap();

        assert_eq!(ra, 10);
        assert_eq!(rb, 20);

        let values = values.borrow();
        assert_eq!(values.iter().filter(|(t, _)| *t == "a").count(), 5);
        assert_eq!(values.iter().filter(|(t, _)| *t == "b").count(), 3);
    });
}

#[test]
fn test_cancel_before_poll_returns_canceled() {
    block_on(async {
        let hit = std::rc::Rc::new(Cell::new(false));
        let hit_task = hit.clone();

        let handle = spawn(async move {
            hit_task.set(true);
            1usize
        });

        handle.cancel();
        let res = handle.await;
        assert!(matches!(res, Err(JoinError::Canceled)));
        assert!(
            !hit.get(),
            "future body should never run after cancel-before-poll"
        );
    });
}

#[test]
fn test_cancel_during_execution_returns_canceled() {
    block_on(async {
        let entered = std::rc::Rc::new(Cell::new(false));
        let done = std::rc::Rc::new(Cell::new(false));

        let entered_task = entered.clone();
        let done_task = done.clone();

        let handle = spawn(async move {
            entered_task.set(true);
            yield_now().await;
            done_task.set(true);
            42usize
        });

        while !entered.get() {
            yield_now().await;
        }

        handle.cancel();
        let res = handle.await;
        assert!(matches!(res, Err(JoinError::Canceled)));
        assert!(
            !done.get(),
            "future should be dropped after cancellation before completing"
        );
    });
}

#[test]
fn test_join_handle_drop_cancels_task() {
    block_on(async {
        let completed = std::rc::Rc::new(Cell::new(false));
        let completed_task = completed.clone();

        let handle = spawn(async move {
            for _ in 0..3 {
                yield_now().await;
            }
            completed_task.set(true);
        });

        drop(handle);

        for _ in 0..8 {
            yield_now().await;
        }

        assert!(
            !completed.get(),
            "dropping JoinHandle should cancel and prevent completion"
        );
    });
}

#[test]
fn test_detach_allows_task_to_continue() {
    block_on(async {
        let completed = std::rc::Rc::new(Cell::new(false));
        let completed_task = completed.clone();

        let handle = spawn(async move {
            for _ in 0..4 {
                yield_now().await;
            }
            completed_task.set(true);
        });

        handle.detach();

        for _ in 0..10 {
            yield_now().await;
        }

        assert!(
            completed.get(),
            "detached task should continue running to completion"
        );
    });
}

#[test]
fn test_multiple_cancels_are_idempotent() {
    block_on(async {
        let ran = std::rc::Rc::new(Cell::new(false));
        let ran_task = ran.clone();

        let handle = spawn(async move {
            ran_task.set(true);
            yield_now().await;
        });

        handle.cancel();
        handle.cancel();
        handle.cancel();

        let res = handle.await;
        assert!(matches!(res, Err(JoinError::Canceled)));
        assert!(!ran.get(), "task should not run after repeated cancels");
    });
}

#[test]
fn test_panic_does_not_affect_other_tasks() {
    block_on(async {
        let run_count = std::rc::Rc::new(Cell::new(0));
        let run_count_task = run_count.clone();

        // This task will panic. We detach it so we aren't waiting on it specifically,
        // but we want to make sure the executor keeps ticking.
        spawn(async {
            panic!("intentional panic");
        })
        .detach();

        // Give the panicked task a chance to run and panic.
        let handle = spawn(async move {
            for _ in 0..5 {
                yield_now().await;
            }
            run_count_task.set(run_count_task.get() + 1);
        });

        handle.await.unwrap();
        assert_eq!(run_count.get(), 1);
    });
}

#[test]
fn test_extra_data() {
    use std::sync::Arc;

    // Create an executor that holds u32 as extra data
    let executor: Executor<u32> = Executor::new();

    let waker_extra = Arc::new(Cell::new(None));
    let waker_extra_task = waker_extra.clone();

    // Create a future that extracts the extra data from its waker
    struct GetExtra {
        waker_extra: Arc<Cell<Option<u32>>>,
        polled: bool,
    }

    impl Future for GetExtra {
        type Output = ();

        fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if !self.polled {
                self.polled = true;
                // Extract the extra data from the waker
                if let Some(&extra) = get_extra::<u32>(cx.waker()) {
                    self.waker_extra.set(Some(extra));
                }
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        }
    }

    let handle = executor.spawn_with(
        GetExtra {
            waker_extra: waker_extra_task,
            polled: false,
        },
        42u32,
    );

    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = pin!(handle);

    // Run the executor until the task completes
    loop {
        if let Poll::Ready(res) = fut.as_mut().poll(&mut cx) {
            res.unwrap();
            break;
        }
        executor.tick();
    }

    // Verify that the extra data was successfully retrieved
    assert_eq!(waker_extra.get(), Some(42u32));
}
