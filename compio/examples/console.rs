//! Observe a compio application with [`tokio-console`].
//!
//! This runs a small echo server on a [`Dispatcher`], so that the console shows
//! the tasks of a thread-per-core application spread over its worker threads,
//! next to a handful of tasks that are wrong on purpose, so that it shows the
//! warnings it can raise about them.
//!
//! Run it with the cfg that lets `console-subscriber` instrument a runtime
//! other than tokio:
//!
//! ```sh
//! RUSTFLAGS="--cfg console_without_tokio_unstable" \
//!     cargo run --example console --features console,time,net,dispatcher,sync
//! ```
//!
//! Then, in another terminal:
//!
//! ```sh
//! tokio-console
//! ```
//!
//! The task list has a column for the thread a task belongs to, and one for its
//! name, which is worth setting for the tasks a user did not spawn themselves,
//! since the location of those points into compio rather than into their code.
//! Press `w` to sort by warnings, or `t` to look at a single task.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console

use std::{
    future::poll_fn, hint::black_box, num::NonZeroUsize, task::Poll, thread, time::Duration,
};

use compio::{
    BufResult,
    dispatcher::Dispatcher,
    io::{AsyncRead, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    runtime::{Runtime, SpawnMeta, spawn_at, spawn_blocking_at},
    time::sleep,
};

/// Worker threads of the dispatcher, each with an executor of its own.
const WORKERS: usize = 4;

fn main() {
    // Install the subscriber before starting any runtime, so that the futures
    // passed to `block_on` show up as tasks as well.
    compio::console_subscriber::init();

    // The tasks that misbehave get threads of their own, both to keep them from
    // disturbing the server and to fill in the thread column. The one that
    // never yields needs one to itself: it never lets its executor run anything
    // else, which is the point of the warning about it.
    let lints = spawn_runtime("compio-lints", lints);
    let hog = spawn_runtime("compio-hog", never_yields);

    Runtime::new().unwrap().block_on(server());

    lints.join().unwrap();
    hog.join().unwrap();
}

/// Run a runtime of its own on a named thread. The console reads the name to
/// tell the executors of a thread-per-core application apart.
///
/// The meta is captured out here rather than in the closure the thread runs,
/// since `#[track_caller]` does not reach into a closure. Without it both of
/// these report the `block_on` below as their location, and only the thread
/// column tells them apart.
#[track_caller]
fn spawn_runtime<F: Future<Output = ()> + 'static>(
    name: &'static str,
    f: fn() -> F,
) -> thread::JoinHandle<()> {
    let meta = SpawnMeta::capture();
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || Runtime::new().unwrap().block_on_at(f(), meta))
        .unwrap()
}

/// Spawn a named task on the current runtime.
///
/// Naming is what [`SpawnMeta`] is for. Without one, the console falls back to
/// the location of the spawn, which this also records.
///
/// A wrapper around a spawn wants `#[track_caller]`, or the location it records
/// is the one inside the wrapper, and every task spawned through it reports the
/// same line.
#[track_caller]
fn spawn(name: &'static str, f: impl Future<Output = ()> + 'static) {
    spawn_at(f, SpawnMeta::capture().named(name)).detach();
}

/// An echo server whose connections are handled by the dispatcher's workers,
/// driven by clients of our own so that there is always something to look at.
async fn server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(WORKERS).unwrap())
        // Named for the same reason the threads above are.
        .thread_names(|i| format!("compio-worker-{i}"))
        .build()
        .unwrap();

    spawn("load-generator", async move {
        loop {
            let mut client = TcpStream::connect(&addr).await.unwrap();
            client.write_all("hello from compio").await.unwrap();
            sleep(Duration::from_millis(50)).await;
        }
    });

    // A task on the blocking pool, which the console reports as busy while the
    // closure runs rather than as idle, and as a `blocking` task rather than as
    // one driven by a future.
    spawn("blocking-pool", async {
        loop {
            spawn_blocking_at(
                || thread::sleep(Duration::from_millis(200)),
                SpawnMeta::capture().named("blocking-pool-work"),
            )
            .await
            .unwrap();
            sleep(Duration::from_millis(300)).await;
        }
    });

    // A task that is idle almost all of the time, for contrast: the console
    // shows where a task's time actually goes.
    spawn("idle-timer", async {
        loop {
            sleep(Duration::from_millis(500)).await;
        }
    });

    println!("run `tokio-console` to watch this process");

    loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Every dispatched closure becomes a task named `dispatch` on whichever
        // worker picks it up. Dropping the receiver only discards the reply,
        // the worker still runs the closure.
        drop(
            dispatcher
                .dispatch(move || async move {
                    let BufResult(read, buf) = stream.read(Vec::with_capacity(32)).await;
                    read.unwrap();
                    stream.write_all(buf).await.unwrap();
                })
                .unwrap(),
        );
    }
}

/// The tasks that the console warns about, other than the one that never
/// yields, which needs a thread to itself.
async fn lints() {
    // Woken by itself three times for every time the timer wakes it, which is
    // over the console's default threshold of half of the wakeups.
    spawn("self-waker", async {
        loop {
            for _ in 0..3 {
                let mut woken = false;
                poll_fn(|cx| {
                    if woken {
                        return Poll::Ready(());
                    }
                    woken = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                })
                .await;
            }
            sleep(Duration::from_millis(100)).await;
        }
    });

    // Pending without ever holding on to the waker, so nothing can ever wake it
    // again. The console counts the live wakers of a task and warns when a task
    // that has not finished has none left.
    spawn("lost-waker", poll_fn(|_| Poll::Pending));

    // A future large enough that the console suggests boxing it: it is measured
    // at the spawn, and this one holds its buffer across an await.
    spawn("large-future", async {
        loop {
            let buf = [0u8; 8192];
            sleep(Duration::from_millis(400)).await;
            black_box(&buf);
        }
    });

    std::future::pending().await
}

/// A task that takes its executor and never gives it back. The console warns
/// about tasks that have been in their first poll for over a second.
async fn never_yields() {
    spawn("never-yields", async {
        loop {
            // Blocking work that never reaches an await point, so the executor
            // cannot take the thread back and the task stays in its very first
            // poll. Sleeping rather than spinning keeps the example from
            // pinning a core for as long as it runs.
            thread::sleep(Duration::from_millis(500));
        }
    });

    std::future::pending().await
}
