use std::{future::Future, marker::PhantomData, sync::Arc};

use async_task::{Runnable, Task};
use compio_driver::NotifyHandle;
use crossbeam_queue::SegQueue;

use crate::runtime::scheduler::{local_queue::LocalQueue, send_wrapper::SendWrapper};

mod local_queue;
mod send_wrapper;

/// A task queue consisting of a local queue and a synchronized queue.
struct TaskQueue {
    local_queue: SendWrapper<LocalQueue<Runnable>>,
    sync_queue: SegQueue<Runnable>,
}

impl TaskQueue {
    /// Creates a new `TaskQueue`.
    fn new() -> Self {
        Self {
            local_queue: SendWrapper::new(LocalQueue::new()),
            sync_queue: SegQueue::new(),
        }
    }

    /// Pushes a `Runnable` task to the appropriate queue.
    ///
    /// If the current thread is the same as the creator thread, push to the
    /// local queue. Otherwise, push to the sync queue.
    fn push(&self, runnable: Runnable, notify: &NotifyHandle) {
        if let Some(local_queue) = self.local_queue.get() {
            local_queue.push(runnable);
            #[cfg(feature = "notify-always")]
            notify.notify().ok();
        } else {
            self.sync_queue.push(runnable);
            notify.notify().ok();
        }
    }

    /// Pops at most one task from each queue and returns them as `(local_task,
    /// sync_task)`.
    ///
    /// # Safety
    ///
    /// Call this method in the same thread as the creator.
    unsafe fn pop(&self) -> (Option<Runnable>, Option<Runnable>) {
        // SAFETY: See the safety comment of this method.
        let local_queue = unsafe { self.local_queue.get_unchecked() };

        let local_task = local_queue.pop();

        // Perform an empty check as a fast path, since `SegQueue::pop()` is more
        // expensive.
        let sync_task = if self.sync_queue.is_empty() {
            None
        } else {
            self.sync_queue.pop()
        };

        (local_task, sync_task)
    }

    /// Returns `true` if both queues are empty.
    ///
    /// # Safety
    ///
    /// Call this method in the same thread as the creator.
    unsafe fn is_empty(&self) -> bool {
        // SAFETY: See the safety comment of this method.
        let local_queue = unsafe { self.local_queue.get_unchecked() };
        local_queue.is_empty() && self.sync_queue.is_empty()
    }
}

/// A scheduler for managing and executing tasks.
pub(crate) struct Scheduler {
    task_queue: Arc<TaskQueue>,
    event_interval: usize,
    // `Scheduler` is `!Send` and `!Sync`.
    _local_marker: PhantomData<*const ()>,
}

impl Scheduler {
    /// Creates a new `Scheduler`.
    pub(crate) fn new(event_interval: usize) -> Self {
        Self {
            task_queue: Arc::new(TaskQueue::new()),
            event_interval,
            _local_marker: PhantomData,
        }
    }

    /// Spawns a new asynchronous task, returning a [`Task`] for it.
    ///
    /// # Safety
    ///
    /// The caller should ensure the captured lifetime long enough.
    pub(crate) unsafe fn spawn_unchecked<F>(
        &self,
        future: F,
        notify: NotifyHandle,
    ) -> Task<F::Output>
    where
        F: Future,
    {
        let schedule = {
            // Use `Weak` to break reference cycle.
            // `TaskQueue` -> `Runnable` -> `TaskQueue`
            let task_queue = Arc::downgrade(&self.task_queue);
            let thread_guard = SendWrapper::new(());

            move |runnable| {
                if let Some(task_queue) = task_queue.upgrade() {
                    task_queue.push(runnable, &notify);
                } else if thread_guard.get().is_none() {
                    // It's not safe to drop the runnable in another thread.
                    std::mem::forget(runnable);
                }
            }
        };

        let (runnable, task) = async_task::spawn_unchecked(future, schedule);
        runnable.schedule();
        task
    }

    /// Run the scheduled tasks.
    ///
    /// The return value indicates whether there are still tasks in the queue.
    pub(crate) fn run(&self) -> bool {
        for _ in 0..self.event_interval {
            // SAFETY:
            // `Scheduler` is `!Send` and `!Sync`, so this method is only called
            // on `TaskQueue`'s creator thread.
            let tasks = unsafe { self.task_queue.pop() };

            // Run the tasks, which will poll the futures.
            // Since spawned tasks are not required to be `Send`, they must always be polled
            // on the same thread. Because `Scheduler` is `!Send` and `!Sync`, this is safe.
            match tasks {
                (Some(local), Some(sync)) => {
                    local.run();
                    sync.run();
                }
                (Some(local), None) => {
                    local.run();
                }
                (None, Some(sync)) => {
                    sync.run();
                }
                (None, None) => break,
            }
        }

        // SAFETY:
        // `Scheduler` is `!Send` and `!Sync`, so this method is only called
        // on `TaskQueue`'s creator thread.
        !unsafe { self.task_queue.is_empty() }
    }

    pub(crate) fn clear(&self) {
        loop {
            // SAFETY:
            // `Scheduler` is `!Send` and `!Sync`, so this method is only called
            // on `TaskQueue`'s creator thread.
            let tasks = unsafe { self.task_queue.pop() };

            if let (None, None) = tasks {
                break;
            }
        }
    }
}
