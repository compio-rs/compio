use std::{
    cell::RefCell,
    future::Future,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Weak},
    task::Waker,
};

use async_task::{Runnable, Task};
use compio_driver::NotifyHandle;
use crossbeam_queue::SegQueue;
use slab::Slab;

use crate::runtime::scheduler::{
    drop_hook::DropHook, local_queue::LocalQueue, send_wrapper::SendWrapper,
};

mod drop_hook;
mod local_queue;
mod send_wrapper;

/// A task queue consisting of a local queue and a synchronized queue.
struct TaskQueue {
    local_queue: Arc<SendWrapper<LocalQueue<Runnable>>>,
    sync_queue: Arc<SegQueue<Runnable>>,
}

impl TaskQueue {
    /// Creates a new `TaskQueue`.
    fn new() -> Self {
        Self {
            local_queue: Arc::new(SendWrapper::new(LocalQueue::new())),
            sync_queue: Arc::new(SegQueue::new()),
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

    /// Clears both queues.
    ///
    /// # Safety
    ///
    /// Call this method in the same thread as the creator.
    unsafe fn clear(&self) {
        // SAFETY: See the safety comment of this method.
        let local_queue = unsafe { self.local_queue.get_unchecked() };

        while let Some(item) = local_queue.pop() {
            drop(item);
        }

        while let Some(item) = self.sync_queue.pop() {
            drop(item);
        }
    }

    /// Downgrades the `TaskQueue` into a `WeakTaskQueue`.
    fn downgrade(&self) -> WeakTaskQueue {
        WeakTaskQueue {
            local_queue: Arc::downgrade(&self.local_queue),
            sync_queue: Arc::downgrade(&self.sync_queue),
            local_thread: self.local_queue.tracker(),
        }
    }
}

/// A weak reference to a `TaskQueue`.
struct WeakTaskQueue {
    local_queue: Weak<SendWrapper<LocalQueue<Runnable>>>,
    sync_queue: Weak<SegQueue<Runnable>>,
    // `()` is a trivial type, so it won't panic on drop even if moved to another thread.
    local_thread: SendWrapper<()>,
}

impl WeakTaskQueue {
    /// Upgrades the `WeakTaskQueue` and pushes the `runnable` into the
    /// appropriate queue.
    fn upgrade_and_push(&self, runnable: Runnable, notify: &NotifyHandle) {
        if self.local_thread.valid() {
            // It's ok to drop the runnable on the same thread.
            if let Some(local_queue) = self.local_queue.upgrade() {
                // SAFETY: already checked
                unsafe { local_queue.get_unchecked() }.push(runnable);
                #[cfg(feature = "notify-always")]
                notify.notify().ok();
            }
        } else if let Some(sync_queue) = self.sync_queue.upgrade() {
            sync_queue.push(runnable);
            notify.notify().ok();
        } else {
            // We have to leak the runnable since it's not safe to drop it on another
            // thread.
            std::mem::forget(runnable);
        }
    }
}

/// A scheduler for managing and executing tasks.
pub(crate) struct Scheduler {
    /// Queue for scheduled tasks.
    task_queue: TaskQueue,

    /// `Waker` of active tasks.
    active_tasks: Rc<RefCell<Slab<Waker>>>,

    /// Number of scheduler ticks for each `run` invocation.
    event_interval: usize,

    /// Makes this type `!Send` and `!Sync`.
    _local_marker: PhantomData<*const ()>,
}

impl Scheduler {
    /// Creates a new `Scheduler`.
    pub(crate) fn new(event_interval: usize) -> Self {
        Self {
            task_queue: TaskQueue::new(),
            active_tasks: Rc::new(RefCell::new(Slab::new())),
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
        let mut active_tasks = self.active_tasks.borrow_mut();
        let task_entry = active_tasks.vacant_entry();

        let future = {
            let active_tasks = self.active_tasks.clone();
            let index = task_entry.key();

            // Wrap the future with a drop hook to remove the waker on drop.
            DropHook::new(future, move || {
                active_tasks.borrow_mut().remove(index);
            })
        };

        let schedule = {
            // The schedule closure is managed by the `Waker` and may be dropped on another
            // thread, so use `WeakTaskQueue` to ensure the `TaskQueue` is always dropped
            // on the creator thread.
            let task_queue = self.task_queue.downgrade();

            move |runnable| task_queue.upgrade_and_push(runnable, &notify)
        };

        let (runnable, task) = async_task::spawn_unchecked(future, schedule);

        // Store the waker.
        task_entry.insert(runnable.waker());

        // Schedule the task for execution.
        runnable.schedule();

        task
    }

    /// Runs the scheduled tasks.
    ///
    /// Returns `true` if there are still tasks in the queue.
    pub(crate) fn run(&self) -> bool {
        for _ in 0..self.event_interval {
            // SAFETY:
            // This method is only called on `TaskQueue`'s creator thread
            // because `Scheduler` is `!Send` and `!Sync`.
            let tasks = unsafe { self.task_queue.pop() };

            // Run the tasks, which will poll the futures.
            //
            // SAFETY:
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
        // This method is only called on `TaskQueue`'s creator thread
        // because `Scheduler` is `!Send` and `!Sync`.
        !unsafe { self.task_queue.is_empty() }
    }

    /// Clears all active tasks.
    ///
    /// This method **must** be called before the scheduler is dropped.
    pub(crate) fn clear(&self) {
        // Wake up all active tasks, which schedules them again.
        self.active_tasks
            .borrow()
            .iter()
            .for_each(|(_, waker)| waker.wake_by_ref());

        // Then drop all scheduled tasks, which drops all futures and removes
        // `Waker`s from `active_tasks` by drop hooks.
        //
        // SAFETY:
        // Since spawned tasks are not required to be `Send`, they must always be
        // dropped on the same thread. Because `Scheduler` is `!Send` and
        // `!Sync`, this is safe.
        //
        // This method is only called on `TaskQueue`'s creator thread
        // because `Scheduler` is `!Send` and `!Sync`.
        unsafe { self.task_queue.clear() };
    }
}
