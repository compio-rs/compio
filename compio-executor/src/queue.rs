use std::{
    fmt::Debug,
    ops::Deref,
    sync::{Arc, Weak},
    task::Waker as StdWaker,
};

use compio_send_wrapper::SendWrapper;
use crossfire::{MTx, Rx, mpsc::Array};
use slotmap::new_key_type;

use crate::{
    PanicResult,
    task::Task,
    util::{Receiver, SlotQueue, oneshot},
    waker::Waker,
};

new_key_type! { pub struct TaskId; }

/// A single-threaded task queue with support for cross-thread wake-ups.
pub struct TaskQueue {
    shared: Arc<Shared>,
    _marker: std::marker::PhantomData<*const ()>,
}

/// A thread-safe handle corresponds to a task.
///
/// This can be used to schedule the task from other threads, but it does not
/// keep the executor alive. If the executor is dropped or the task is dropped,
/// [`schedule`] will fail and return `false`.
///
/// [`schedule`]: Handle::schedule
#[derive(Debug, Clone)]
pub struct Handle {
    id: TaskId,
    shared: Weak<Shared>,
}

#[derive(Debug)]
struct Shared {
    waker: Option<StdWaker>,
    local: SendWrapper<Local>,
    sync_tx: MTx<Array<TaskId>>,
}

/// Local part of the shared state, which is not thread-safe and only accessed
/// by the creating thread.
#[derive(Debug)]
struct Local {
    queue: SlotQueue<TaskId, Task>,
    sync_rx: Rx<Array<TaskId>>,
}

/// SlotQueue will not be accessed cross thread, and SendWrapper ensures Drop
/// will only be called on the creating thread.
unsafe impl Sync for Shared {}

const _: () = {
    const fn is_mt<T: Send + Sync>() {}

    is_mt::<Shared>();
};

impl TaskQueue {
    /// Create a new task queue.
    pub fn new(waker: Option<StdWaker>, sync_size: usize, local_size: usize) -> Self {
        let (sync_tx, sync_rx) = crossfire::mpsc::bounded_blocking(sync_size);
        let local = Local {
            queue: SlotQueue::new(local_size),
            sync_rx,
        };
        let inner = Shared {
            waker,
            local: SendWrapper::new(local),
            sync_tx,
        };
        Self {
            shared: Arc::new(inner),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn clear(&self) {
        self.local().queue.clear();
    }

    pub fn has_hot(&self) -> bool {
        self.local().hot_head().is_some()
    }

    /// Create a handle for a task.
    pub fn handle(&self, id: TaskId) -> Handle {
        Handle {
            id,
            shared: Arc::downgrade(&self.shared),
        }
    }

    /// Create a waker for a task.
    pub fn waker<E: Send + Sync>(&self, id: TaskId, extra: E) -> Waker<E> {
        Waker::new(self.handle(id), extra)
    }

    /// Flush the sync queue to the local queue.
    pub fn flush_sync(&self) {
        let queue = self.local();
        while let Ok(id) = queue.sync_rx.try_recv() {
            queue.make_hot(id);
        }
    }

    fn local(&self) -> &Local {
        // SAFETY: TaskQueue is !Send and !Sync
        unsafe { self.shared.local.get_unchecked() }
    }

    /// Iterate over the tasks in the queue.
    pub fn iter(&self, cold_limit: u32) -> impl Iterator<Item = TaskId> {
        let local = self.local();
        local
            .iter_hot()
            .chain(local.iter_cold().take(cold_limit as _))
    }

    /// Push a future into the queue.
    pub fn push<F: Future + 'static, E: Send + Sync>(
        &self,
        fut: F,
        extra: E,
    ) -> (TaskId, Receiver<PanicResult<F::Output>>) {
        let (tx, rx) = oneshot();
        let id = self.local().push_back_with(|id| {
            let waker = self.waker(id, extra);
            Task::new(fut, tx, waker.into_std())
        });
        (id, rx)
    }

    /// Run a task.
    pub fn run(&self, id: TaskId) {
        let queue = self.local();

        let inner = match unsafe { queue.get(id) } {
            Some(task) => task.take().expect("Inner was not reset"),
            None => return,
        };

        queue.make_cold(id);
        match inner.poll() {
            Some(inner) => {
                unsafe { queue.get(id) }
                    .expect("Task removed during run")
                    .reset(inner);
            }
            None => {
                queue.remove(id);
            }
        }
    }
}

impl Debug for TaskQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskQueue")
            .field("shared", &self.shared)
            .finish()
    }
}

impl Deref for Local {
    type Target = SlotQueue<TaskId, Task>;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

impl Shared {
    /// Schedule a task for execution.
    ///
    /// Returns `true` if the task was scheduled successfully, or `false`
    /// otherwise, due to the executor being dropped.
    fn schedule(&self, id: TaskId) -> bool {
        if let Some(local) = self.local.get() {
            // piggyback multi-thread wake-ups
            while let Ok(id) = local.sync_rx.try_recv() {
                local.make_hot(id);
            }
            local.make_hot(id);
            #[cfg(feature = "notify-always")]
            if let Some(w) = self.waker.as_ref() {
                w.wake_by_ref()
            }
            true
        } else {
            let res = self.sync_tx.send(id).is_ok();
            if let Some(w) = self.waker.as_ref() {
                w.wake_by_ref()
            }
            res
        }
    }
}

impl Handle {
    /// Enqueues the task for execution.
    ///
    /// Returns `true` if the task was enqueued successfully, or `false`
    /// otherwise, due to the executor being dropped.
    pub fn schedule(&self) -> bool {
        self.shared.upgrade().is_some_and(|q| q.schedule(self.id))
    }

    /// Check if the handle is at the same thread as the executor.
    pub fn is_local(&self) -> bool {
        self.shared.upgrade().is_some_and(|q| q.local.valid())
    }
}
