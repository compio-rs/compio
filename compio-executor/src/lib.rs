//! Executor for compio runtime.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

use std::{any::Any, fmt::Debug, ptr::NonNull, task::Waker};

use crate::queue::{TaskId, TaskQueue};

mod join_handle;
mod queue;
mod task;
mod util;
mod waker;

use compio_send_wrapper::SendWrapper;
use crossbeam_queue::ArrayQueue;
pub use join_handle::{JoinError, JoinHandle, ResumeUnwind};

pub(crate) type PanicResult<T> = Result<T, Panic>;
pub(crate) type Panic = Box<dyn Any + Send + 'static>;

/// A dual-queue executor optimized for singlethreaded usecase, with support for
/// multithreaded wakes.
///
/// Same-thread wakes ([`Waker::wake`]) will schedule tasks within the queue
/// directly; cross-thread wakes will send task id's to a channel, and
/// piggybacked to singlethreaded wakes or ticks. This ensures maximum
/// performance for singlethreaded scenario at the trade-off of worse tail
/// latency for multithreaded wake-ups.
///
/// Optionally, all [`Waker`]s generated from this executor can contain an extra
/// data, parameterized as `E`.
///
/// [`Waker`]: std::task::Waker
/// [`Waker::wake`]: std::task::Waker::wake
#[derive(Debug)]
pub struct Executor {
    ptr: NonNull<Shared>,
    config: ExecutorConfig,
}

/// Configuration for [`Executor`].
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// The size of the sync queue, which holds task id's for cross-thread
    /// wakes.
    ///
    /// This is fixed and will create backpressure when full.
    pub sync_queue_size: usize,

    /// The size of the local queues, which hold tasks for same-thread
    /// execution.
    ///
    /// This is dynamically resized to avoid blocking.
    pub local_queue_size: usize,

    /// The maximum number of hot tasks to run in each tick.
    pub max_interval: u32,

    /// A waker to be waken when a task is scheduled from other thread.
    ///
    /// This is useful for waking up drivers that switch to kernel state when
    /// idle.
    ///
    /// Enable `notify-always` feature to wake this waker on every schedule,
    /// even if the executor is already awake.
    pub waker: Option<Waker>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            sync_queue_size: 64,
            local_queue_size: 63,
            max_interval: 32,
            waker: None,
        }
    }
}

pub(crate) struct Shared {
    waker: Option<Waker>,
    sync: ArrayQueue<TaskId>,
    queue: SendWrapper<TaskQueue>,
}

impl Shared {
    pub fn new(config: &ExecutorConfig) -> Self {
        Self {
            waker: None,
            sync: ArrayQueue::new(config.sync_queue_size),
            queue: SendWrapper::new(TaskQueue::new(config.local_queue_size)),
        }
    }
}

impl Executor {
    /// Create a new executor.
    pub fn new() -> Self {
        Self::with_config(ExecutorConfig::default())
    }

    /// Create a new executor with config.
    pub fn with_config(config: ExecutorConfig) -> Self {
        let ptr = Box::into_raw(Box::new(Shared::new(&config)));

        Self {
            config,
            ptr: unsafe { NonNull::new_unchecked(ptr) },
        }
    }

    /// Spawn a future onto the executor.
    pub fn spawn<F: Future + 'static>(&self, fut: F) -> JoinHandle<F::Output> {
        let shared = self.shared();
        let tracker = shared.queue.tracker();
        // SAFETY: Executor cannot be sent to ther thread
        let queue = unsafe { shared.queue.get_unchecked() };
        let task = queue.insert(self.ptr, tracker, fut);

        JoinHandle::new(task)
    }

    /// Retrieve all sync tasks, schedule those to the tail of `hot` queue
    /// and run at most [`max_interval`] tasks.
    ///
    /// Running start with `hot` tasks, then `cold` ones. Finished tasks will
    /// be pushed back to tail of `cold` queue.
    ///
    /// Return whether there are still hot tasks after the tick.
    ///
    /// [`max_interval`]: ExecutorConfig::max_interval
    pub fn tick(&self) -> bool {
        let queue = self.queue();

        while let Some(id) = self.shared().sync.pop() {
            queue.make_hot(id);
        }

        for id in queue.iter_hot().take(self.config.max_interval as _) {
            queue.make_cold(id);
            let task = queue.take(id).expect("Task was not reset back");
            let res = unsafe { task.run() };
            if res.is_ready() {
                // SAFETY: We're removing it soon, so drop will only be called once.
                unsafe { task.drop() };
                queue.remove(id);
            } else {
                queue.reset(id, task);
            }
        }

        queue.has_hot()
    }

    /// Check if there's still scheduled task that needs to be ran.
    pub fn has_task(&self) -> bool {
        self.queue().hot_head().is_some()
    }

    /// Clear the executor, drop all tasks.
    pub fn clear(&self) {
        while self.shared().sync.pop().is_some() {}
        unsafe { self.queue().clear() };
    }

    #[inline(always)]
    fn shared(&self) -> &Shared {
        unsafe { self.ptr.as_ref() }
    }

    #[inline(always)]
    fn queue(&self) -> &TaskQueue {
        // SAFETY: Executor is single threaded
        unsafe { self.shared().queue.get_unchecked() }
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.clear();
        unsafe { drop(Box::from_raw(self.ptr.as_ptr())) };
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}
