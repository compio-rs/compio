use futures_util::task::AtomicWaker;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    thread::{self, ThreadId},
};

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
