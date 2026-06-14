use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    thread::{self, ThreadId},
    time::Duration,
};

use compio_runtime::CancelToken;
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

#[test]
fn test_task_dropped_when_runtime_drops() {
    struct DropFlag(Rc<Cell<bool>>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let flag = Rc::new(Cell::new(false));
    let flag2 = flag.clone();

    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        compio_runtime::spawn(async move {
            let _guard = DropFlag(flag2);

            // `CancelToken` contains a strong reference to the driver, but should not
            // prevent the task from being dropped when the runtime is dropped.
            let _token = CancelToken::new();

            compio_runtime::time::sleep(std::time::Duration::from_secs(3600)).await;
        })
        .detach();
    });
    drop(rt);

    assert!(flag.get(), "spawned task was not dropped: Rc cycle?");
}

#[test]
fn test_run_enters_runtime_context() {
    let rt = compio_runtime::Runtime::new().unwrap();
    let waker = Arc::new(AtomicWaker::new());
    let waker2 = waker.clone();

    rt.block_on(async {
        compio_runtime::spawn(async move {
            let timer = compio_runtime::time::sleep(Duration::from_secs(3600));
            futures_util::pin_mut!(timer);

            std::future::poll_fn(|cx| {
                let _ = timer.as_mut().poll(cx);
                waker2.register(cx.waker());
                Poll::<()>::Pending
            })
            .await;
        })
        .detach();

        compio_runtime::spawn(async {}).await.unwrap();
    });

    waker.take().unwrap().wake();

    // Must not panic: run() should set CURRENT_RUNTIME so that
    // TimerFuture::poll (and TimerFuture::drop) can find the runtime.
    rt.run();
}
