//! The runtime of compio.
//! We don't expose the runtime struct because there could be only one runtime
//! in each thread.
//!
//! ```
//! let ans = compio::task::block_on(async {
//!     println!("Hello world!");
//!     42
//! });
//! assert_eq!(ans, 42);
//! ```

#[cfg(feature = "time")]
pub mod time;

mod key;
pub(crate) mod runtime;

use std::{future::Future, io};

use async_task::Task;
use compio_buf::BufResult;
use compio_driver::{OpCode, RawFd};
pub(crate) use key::Key;
use runtime::Runtime;

thread_local! {
    pub(crate) static RUNTIME: Runtime = Runtime::new().expect("cannot create compio runtime");
}

/// Start a compio runtime and block on the future till it completes.
///
/// ```
/// compio::task::block_on(async {
///     // Open a file
///     let file = compio::fs::File::open("Cargo.toml").unwrap();
///
///     let buf = Vec::with_capacity(4096);
///     // Read some data, the buffer is passed by ownership and
///     // submitted to the kernel. When the operation completes,
///     // we get the buffer back.
///     let (res, buf) = file.read_at(buf, 0).await;
///     let n = res.unwrap();
///     assert_eq!(n, buf.len());
///
///     // Display the contents
///     println!("{:?}", &buf);
/// })
/// ```
pub fn block_on<F: Future>(future: F) -> F::Output {
    RUNTIME.with(|runtime| runtime.block_on(future))
}

/// Spawns a new asynchronous task, returning a [`Task`] for it.
///
/// Spawning a task enables the task to execute concurrently to other tasks.
/// There is no guarantee that a spawned task will execute to completion.
///
/// ```
/// compio::task::block_on(async {
///     let task = compio::task::spawn(async {
///         println!("Hello from a spawned task!");
///         42
///     });
///
///     assert_eq!(task.await, 42);
/// })
/// ```
pub fn spawn<F: Future + 'static>(future: F) -> Task<F::Output> {
    RUNTIME.with(|runtime| runtime.spawn(future))
}

/// Attach a raw file descriptor/handle/socket to the runtime.
///
/// You only need this when authoring your own high-level APIs. High-level
/// resources in this crate are attached automatically.
pub fn attach(fd: RawFd) -> io::Result<()> {
    RUNTIME.with(|runtime| runtime.attach(fd))
}

/// Submit an operation to the runtime.
///
/// You only need this when authoring your own [`OpCode`].
pub fn submit<T: OpCode + 'static>(op: T) -> impl Future<Output = BufResult<usize, T>> {
    RUNTIME.with(|runtime| runtime.submit(op))
}
