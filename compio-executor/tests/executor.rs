use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use compio_executor::{Executor, JoinError, JoinHandle};

std::thread_local! {
    static EXE: Executor = Executor::new();
}

fn setup_log() {
    #[cfg(feature = "enable_log")]
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
}

fn spawn<F: Future + 'static>(f: F) -> JoinHandle<F::Output> {
    EXE.with(|exe| exe.spawn(f))
}

fn block_on<F: Future + 'static>(f: F) -> F::Output {
    EXE.with(|exe| {
        let cx = &mut Context::from_waker(Waker::noop());
        let mut f = pin!(f);
        loop {
            if let Poll::Ready(res) = f.as_mut().poll(cx) {
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
    setup_log();

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
    setup_log();

    block_on(async {
        let hit = std::rc::Rc::new(Cell::new(false));
        let hit_task = hit.clone();

        let handle = spawn(async move {
            hit_task.set(true);
            1usize
        });

        let res = handle.cancel().await;

        assert!(res.is_none());
        assert!(
            !hit.get(),
            "future body should never run after cancel-before-poll"
        );
    });
}

#[test]
fn test_cancel_during_execution_returns_canceled() {
    setup_log();

    let handle = spawn(std::future::pending::<()>());

    // Poll the task once before cancelling it.
    EXE.with(|exe| exe.tick());
    assert!(!handle.is_finished());

    assert!(block_on(handle.cancel()).is_none());
}

#[test]
fn test_join_handle_drop_cancels_task() {
    setup_log();

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
    setup_log();

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
fn test_panic_does_not_affect_other_tasks() {
    setup_log();

    block_on(async {
        let run_count = std::rc::Rc::new(Cell::new(0));
        let run_count_task = run_count.clone();

        // This task will panic. We detach it so we aren't waiting on it
        // specifically, but we want to make sure the executor keeps
        // ticking.
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
fn test_join_result_resume_unwind() {
    setup_log();

    block_on(async {
        let handle: JoinHandle<()> = spawn(async {
            panic!("resume_unwind panic");
        });

        let JoinError::Panicked(res) = handle.await.unwrap_err() else {
            unreachable!("Future panicked")
        };

        let Some(msg) = res.downcast_ref::<&'static str>() else {
            unreachable!("Panic payload should be a static string")
        };

        assert_eq!(*msg, "resume_unwind panic");
    });
}

#[test]
fn test_num_alive_tasks() {
    setup_log();

    let exe = Executor::new();
    assert_eq!(exe.num_alive_tasks(), 0);

    // A task is alive as soon as it is spawned, before it ever runs.
    let finishes = exe.spawn(async {});
    let pending = exe.spawn(std::future::pending::<()>());
    let cancelled = exe.spawn(std::future::pending::<()>());
    assert_eq!(exe.num_alive_tasks(), 3);

    // The finished task is gone, the ones waiting to be woken still count.
    exe.tick();
    assert!(finishes.is_finished());
    assert_eq!(exe.num_alive_tasks(), 2);

    // Detaching does not change the count, the task keeps running.
    pending.detach();
    assert_eq!(exe.num_alive_tasks(), 2);

    // Dropping the handle cancels the task; it is removed when the
    // executor processes the cancellation.
    drop(cancelled);
    exe.tick();
    assert_eq!(exe.num_alive_tasks(), 1);

    exe.clear();
    assert_eq!(exe.num_alive_tasks(), 0);
}

#[test]
fn test_clear_with_join_handle_in_task() {
    setup_log();

    let exe = std::rc::Rc::new(Executor::new());
    let inner = exe.clone();
    exe.spawn(async move {
        let _ = inner.spawn(std::future::pending::<()>()).await;
    })
    .detach();
    exe.tick();
    assert_eq!(exe.num_alive_tasks(), 2);

    exe.clear();
    assert_eq!(exe.num_alive_tasks(), 0);
}

#[test]
fn test_clear_with_spawn_on_drop() {
    setup_log();

    struct SpawnOnDrop(std::rc::Rc<Executor>);

    impl Drop for SpawnOnDrop {
        fn drop(&mut self) {
            self.0.spawn(std::future::pending::<()>()).detach();
        }
    }

    let exe = std::rc::Rc::new(Executor::new());
    let guard = SpawnOnDrop(exe.clone());
    exe.spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    })
    .detach();
    exe.tick();
    assert_eq!(exe.num_alive_tasks(), 1);

    exe.clear();
    assert_eq!(exe.num_alive_tasks(), 0);
}
