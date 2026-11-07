use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    thread::{self, ThreadId},
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

/// Dropping the runtime must end every in-flight op: a receive left pending in
/// the kernel would consume data sent afterwards and write it into the freed
/// buffer. A duplicate of the driver fd (as a `fork` without `exec` leaves)
/// keeps an io_uring ring alive after the runtime closes its own fd.
#[cfg(unix)]
#[test]
fn test_drop_ends_pending_recv() {
    use std::{
        io::{Read, Write},
        os::{
            fd::{AsFd, AsRawFd, BorrowedFd},
            unix::net::UnixStream,
        },
    };

    use compio_driver::{AsSocketState, op::Recv};
    use rustix::net::RecvFlags;

    struct Socket(UnixStream);

    impl AsFd for Socket {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }
    }

    impl AsSocketState for Socket {}

    let (mut local, mut peer) = UnixStream::pair().unwrap();
    // The poll driver tries the receive inline: it must not block.
    local.set_nonblocking(true).unwrap();
    let rt = compio_runtime::Runtime::new().unwrap();
    // SAFETY: the driver fd is open until `rt` is dropped.
    let _ring = rustix::io::dup(unsafe { BorrowedFd::borrow_raw(rt.as_raw_fd()) }).unwrap();

    let socket = Socket(local.try_clone().unwrap());
    rt.block_on(async move {
        compio_runtime::spawn(compio_runtime::submit(Recv::new(
            socket,
            Vec::with_capacity(64),
            RecvFlags::empty(),
        )))
        .detach();
        // Let the task push the receive, and the driver submit it.
        compio_runtime::time::sleep(std::time::Duration::from_millis(10)).await;
    });
    drop(rt);

    peer.write_all(b"x").unwrap();
    let mut buf = [0; 1];
    assert_eq!(
        local
            .read(&mut buf)
            .expect("data consumed by a receive that outlived the runtime"),
        1
    );
    assert_eq!(&buf, b"x");
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

            // `CancelToken` contains a strong reference to the driver, but
            // should not prevent the task from being dropped when
            // the runtime is dropped.
            let _token = CancelToken::new();

            compio_runtime::time::sleep(std::time::Duration::from_secs(3600)).await;
        })
        .detach();
    });
    drop(rt);

    assert!(flag.get(), "spawned task was not dropped: Rc cycle?");
}
