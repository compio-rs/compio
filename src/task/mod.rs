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

#[cfg(not(feature = "once_cell"))]
use std::cell::OnceCell as LazyCell;
use std::future::Future;

use async_task::Task;
#[cfg(feature = "once_cell")]
use once_cell::unsync::Lazy as LazyCell;

mod runtime;
use runtime::Runtime;

mod op;
#[cfg(feature = "time")]
mod time;

thread_local! {
    pub(crate) static RUNTIME: LazyCell<Runtime> = LazyCell::new(|| Runtime::new().unwrap());
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
