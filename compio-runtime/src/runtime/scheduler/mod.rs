use std::{cell::RefCell, future::Future, marker::PhantomData, rc::Rc, task::Waker};

use async_task::{Runnable, Task};
use compio_driver::NotifyHandle;
use crossbeam_queue::SegQueue;
use slab::Slab;

use crate::runtime::scheduler::{
    drop_hook::DropHook, local_queue::LocalQueue, raw_ref::Own, send_wrapper::SendWrapper,
};

mod drop_hook;
mod local_queue;
mod raw_ref;
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
}

/// A scheduler for managing and executing tasks.
pub(crate) struct Scheduler {
    /// Queue for scheduled tasks.
    task_queue: Own<TaskQueue>,

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
            task_queue: Own::new(TaskQueue::new()),
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
            let task_queue = self.task_queue.raw_ref();

            move |runnable| {
                // SAFETY:
                // `Scheduler` drains and drops all `Runnable`s before it is dropped.
                // If this closure is still invoked and can push a `Runnable`, the
                // `Scheduler` is necessarily still alive, so the `task_queue` reference
                // obtained from `raw_ref()` is valid.
                unsafe { task_queue.as_ref() }.push(runnable, &notify);
            }
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
