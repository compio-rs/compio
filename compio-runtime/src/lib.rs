//! The runtime of compio.
//! We don't expose the runtime struct because there could be only one runtime
//! in each thread.
//!
//! ```
//! let ans = compio_runtime::block_on(async {
//!     println!("Hello world!");
//!     42
//! });
//! assert_eq!(ans, 42);
//! ```

#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

mod attacher;
mod key;
pub(crate) mod runtime;

#[cfg(feature = "event")]
pub mod event;
#[cfg(feature = "time")]
pub mod time;

use std::{cell::RefCell, future::Future, io};

use async_task::Task;
pub use attacher::*;
use compio_buf::BufResult;
use compio_driver::{OpCode, ProactorBuilder, RawFd};
pub(crate) use key::Key;
use runtime::Runtime;

thread_local! {
    pub(crate) static PROACTOR_BUILDER: RefCell<ProactorBuilder> = RefCell::new(ProactorBuilder::new());
    pub(crate) static RUNTIME: Runtime = PROACTOR_BUILDER.with(|builder| Runtime::new(&builder.borrow())).expect("cannot create compio runtime");
}

/// Config the inner proactor with a [`ProactorBuilder`]. Note that if any
/// runtime related method is called before, there will be no influence.
pub fn config_proactor(new_builder: ProactorBuilder) {
    PROACTOR_BUILDER.with(|builder| *builder.borrow_mut() = new_builder);
}

/// Start a compio runtime and block on the future till it completes.
pub fn block_on<F: Future>(future: F) -> F::Output {
    RUNTIME.with(|runtime| runtime.block_on(future))
}

/// Spawns a new asynchronous task, returning a [`Task`] for it.
///
/// Spawning a task enables the task to execute concurrently to other tasks.
/// There is no guarantee that a spawned task will execute to completion.
///
/// ```
/// compio_runtime::block_on(async {
///     let task = compio_runtime::spawn(async {
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
