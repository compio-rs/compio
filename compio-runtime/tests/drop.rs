use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::SeqCst},
    },
    task::{Context, Poll},
    thread::{self, ThreadId},
};

use futures_util::task::AtomicWaker;

struct DropWatcher {
    waker: Arc<AtomicWaker>,
    thread_id: ThreadId,
}

impl DropWatcher {
    fn new(waker: Arc<AtomicWaker>) -> Self {
        Self {
            waker,
            thread_id: thread::current().id(),
        }
    }
}

impl Future for DropWatcher {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.waker.register(cx.waker());
        Poll::Pending
    }
}

impl Drop for DropWatcher {
    fn drop(&mut self) {
        if self.thread_id != thread::current().id() {
            panic!("DropWatcher dropped on a different thread!!!");
        }
    }
}

#[test]
fn test_drop_with_timer() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        compio_runtime::spawn(async {
            loop {
                compio_runtime::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        })
        .detach();
    })
}

/// Regression test for the `Rc` cycle that prevented `executor.clear()` from
/// running when a spawned task parked on `sleep` (or any other future that
/// stores a `CancelToken`/`Submit`/`SubmitMulti`).
///
/// Before the fix, `CancelToken::Inner` held a strong `Runtime` clone.  A
/// parked task therefore formed:
///   task → CancelToken → Rc<RuntimeInner> → executor → task
/// `Runtime::drop` saw `strong_count > 1` and early-returned, so
/// `executor.clear()` never ran, the task was never dropped, and the
/// io_uring fd (plus every socket fd owned by in-flight ops) leaked for
/// the life of the process.
///
/// After the fix, `CancelToken::Inner` (and `Submit`/`SubmitMulti`) hold
/// `Weak<RuntimeInner>`, so `strong_count` is always 1 when the last user
/// `Runtime` drops, `executor.clear()` always runs, and tasks are dropped.
#[test]
#[cfg(feature = "time")]
fn test_task_dropped_when_runtime_drops() {
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, SeqCst);
        }
    }

    let flag = Arc::new(AtomicBool::new(false));
    let flag2 = flag.clone();

    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        compio_runtime::spawn(async move {
            let _guard = DropFlag(flag2);
            // Parking on sleep is the exact pattern that formed the Rc cycle:
            // CancelToken held a strong Runtime, keeping the executor alive,
            // keeping the task alive, keeping the CancelToken alive, ...
            compio_runtime::time::sleep(std::time::Duration::from_secs(3600)).await;
        })
        .detach();
        // `block_on` calls `self.run()` once after the main future resolves,
        // which is enough to poll the spawned task and park it on the timer.
        // No explicit yield needed.
    });
    drop(rt);

    assert!(
        flag.load(SeqCst),
        "spawned task was not dropped: Rc cycle still present, executor.clear() never ran"
    );
}

#[test]
fn test_wake_after_runtime_drop() {
    let waker = Arc::new(AtomicWaker::new());
    let waker_clone = waker.clone();

    let rt = compio_runtime::Runtime::new().unwrap();

    rt.block_on(async move {
        compio_runtime::spawn(DropWatcher::new(waker_clone)).detach();
    });

    drop(rt);

    // Use `unwrap()` to ensure there is a waker stored.
    waker.take().unwrap().wake();
}

#[test]
fn test_wake_from_another_thread_after_runtime_drop() {
    let waker = Arc::new(AtomicWaker::new());
    let waker_clone = waker.clone();

    let rt = compio_runtime::Runtime::new().unwrap();

    rt.block_on(async move {
        compio_runtime::spawn(DropWatcher::new(waker_clone)).detach();
    });

    drop(rt);

    thread::spawn(move || {
        // Use `unwrap()` to ensure there is a waker stored.
        waker.take().unwrap().wake();
    })
    .join()
    .unwrap();
}
