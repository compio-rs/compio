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

use std::{
    any::Any,
    fmt::Debug,
    marker::PhantomData,
    mem::ManuallyDrop,
    panic::resume_unwind,
    ptr,
    task::{Context, Poll, Waker as StdWaker},
};

pub use crate::waker::get_extra;
use crate::{
    queue::{Handle, TaskQueue},
    util::Receiver,
};

mod queue;
mod task;
mod util;
mod waker;

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
pub struct Executor<E = ()> {
    queue: TaskQueue,
    config: ExecutorConfig,
    _marker: PhantomData<E>,
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
    /// The maximum number of tasks to run in each tick.
    ///
    /// This includes both hot and cold tasks.
    pub max_interval: u32,
    /// The maximum number of cold tasks to run in each tick.
    ///
    /// By default, each tick `max_interval - num_hot` number of cold tasks will
    /// be ran. This limits the number of cold tasks can be ran even if
    /// `max_interval` is not reached. This will be ignored if `max_interval` is
    /// reached first.
    pub max_cold_interval: u32,
    /// A waker to be waken when a task is scheduled from other thread.
    ///
    /// This is useful for waking up drivers that switchs to kernel state when
    /// idle.
    ///
    /// Enable `notify-always` feature to wake this waker on every schedule,
    /// even if the executor is already awake.
    pub waker: Option<StdWaker>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            sync_queue_size: 64,
            local_queue_size: 63,
            max_interval: 32,
            max_cold_interval: u32::MAX,
            waker: None,
        }
    }
}

impl<E> Executor<E> {
    /// Create a new executor.
    pub fn new() -> Self {
        Self::with_config(ExecutorConfig::default())
    }

    /// Create a new executor with config.
    pub fn with_config(mut config: ExecutorConfig) -> Self {
        Self {
            queue: TaskQueue::new(
                config.waker.take(),
                config.sync_queue_size,
                config.local_queue_size,
            ),
            config,
            _marker: PhantomData,
        }
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
        self.queue.flush_sync();
        for id in self
            .queue
            .iter(self.config.max_cold_interval)
            .take(self.config.max_interval as _)
        {
            self.queue.run(id);
        }
        self.queue.has_hot()
    }

    /// Clear the executor, drop all tasks.
    pub fn clear(&self) {
        self.queue.clear();
    }
}

impl<E: Send + Sync> Executor<E> {
    /// Spawn a future onto the executor.
    pub fn spawn_with<F: Future + 'static>(&self, fut: F, extra: E) -> JoinHandle<F::Output> {
        let (id, rx) = self.queue.push(fut, extra);

        JoinHandle {
            handle: self.queue.handle(id),
            rx,
        }
    }
}

impl<E: Default + Send + Sync> Executor<E> {
    /// Spawn a future onto the executor.
    pub fn spawn<F: Future + 'static>(&self, fut: F) -> JoinHandle<F::Output> {
        self.spawn_with(fut, E::default())
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

/// A handle that awaits the result of a task.
///
/// Dropping a [`JoinHandle`] will cancel the task. To run the task in the
/// background, use [`JoinHandle::detach`].
#[must_use = "Drop `JoinHandle` will cancel the task. Use `detach` to run it in background."]
#[derive(Debug)]
pub struct JoinHandle<T> {
    handle: Handle,
    rx: Receiver<PanicResult<T>>,
}

impl<T> Unpin for JoinHandle<T> {}

impl<T> JoinHandle<T> {
    /// Cancel the task.
    pub async fn cancel(self) -> Option<T> {
        self.rx.set_canceled();
        self.handle.schedule();
        self.await.ok()
    }

    /// Check if the task is canceled.
    pub fn is_canceled(&self) -> bool {
        self.rx.is_canceled()
    }

    /// Detach the task to let it run in the background.
    pub fn detach(self) {
        let this = ManuallyDrop::new(self);
        unsafe {
            _ = ptr::read(&this.rx);
            _ = ptr::read(&this.handle)
        };
    }
}

/// Task failed to execute to completion.
#[derive(Debug)]
pub enum JoinError {
    /// The task was canceled.
    Canceled,
    /// The task panicked.
    Panicked(Panic),
}

/// Trait to resume unwind from a [`JoinError`].
pub trait ResumeUnwind {
    /// The output type.
    type Output;

    /// Resume the panic if the task panicked.
    fn resume_unwind(self) -> Self::Output;
}

impl<T> ResumeUnwind for Result<T, JoinError> {
    type Output = Option<T>;

    fn resume_unwind(self) -> Self::Output {
        match self {
            Ok(res) => Some(res),
            Err(JoinError::Canceled) => None,
            Err(JoinError::Panicked(e)) => resume_unwind(e),
        }
    }
}

impl JoinError {
    /// Resume unwind if the task panicked, otherwise do nothing.
    pub fn resume_unwind(self) {
        if let JoinError::Panicked(e) = self {
            resume_unwind(e)
        }
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let is_local = self.handle.is_local();
        let res = if is_local {
            unsafe { self.rx.poll_local(cx) }
        } else {
            self.rx.poll(cx)
        };
        match res {
            Poll::Pending => {
                if self.handle.schedule() {
                    Poll::Pending
                } else {
                    Poll::Ready(Err(JoinError::Canceled))
                }
            }
            Poll::Ready(Some(Ok(res))) => Poll::Ready(Ok(res)),
            Poll::Ready(Some(Err(err))) => Poll::Ready(Err(JoinError::Panicked(err))),
            Poll::Ready(None) => Poll::Ready(Err(JoinError::Canceled)),
        }
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        self.rx.set_canceled();
    }
}
