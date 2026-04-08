use std::{fmt::Debug, ptr::NonNull};

use compio_send_wrapper::SendWrapper;
use slotmap::new_key_type;

use crate::{Shared, task::Task, util::assert_not_impl};

new_key_type! { pub struct TaskId; }

use compio_log::instrument;
use slotmap::SlotMap;

use crate::UnsafeCell;

/// A single-threaded dual queue (hot and cold) for scheduling tasks.
pub struct TaskQueue {
    inner: UnsafeCell<Inner>,
}

assert_not_impl!(TaskQueue, Send);
assert_not_impl!(TaskQueue, Sync);

impl Debug for TaskQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            self.with_inner(|inner| {
                f.debug_struct("TaskQueue")
                    .field("map", &inner.map)
                    .field("hot", &inner.hot)
                    .field("cold", &inner.cold)
                    .finish()
            })
        }
    }
}

#[derive(Debug)]
struct Inner {
    map: SlotMap<TaskId, Item>,
    hot: List,
    cold: List,
}

#[derive(Debug, Clone, Copy, Default)]
struct List {
    head: Option<TaskId>,
    tail: Option<TaskId>,
}

#[derive(Debug)]
struct Item {
    prev: Option<TaskId>,
    next: Option<TaskId>,
    task: Option<Task>,
    is_hot: bool,
}

#[derive(Debug)]
pub struct Iter<'a> {
    queue: &'a TaskQueue,
    curr: Option<TaskId>,
}

type QueueMarker = bool;
const HOT: QueueMarker = true;
const COLD: QueueMarker = false;

impl TaskQueue {
    pub fn new(size: usize) -> Self {
        Self {
            inner: UnsafeCell::new(Inner::new(size)),
        }
    }

    /// Clear the map.
    ///
    /// # Safety
    ///
    /// Must only be called by `Executor`.
    pub unsafe fn clear(&self) {
        instrument!(compio_log::Level::DEBUG, "clear");

        unsafe {
            self.with_inner(|inner| {
                if inner.map.is_empty() {
                    return;
                }

                inner.hot.head = None;
                inner.hot.tail = None;
                inner.cold.head = None;
                inner.cold.tail = None;

                for task in inner.map.drain().filter_map(|(_, i)| i.task) {
                    task.unset_shared();
                    task.drop();
                }

                debug_assert!(inner.map.is_empty());
            })
        }
    }

    pub fn has_hot(&self) -> bool {
        self.hot_head().is_some()
    }

    pub fn take(&self, key: TaskId) -> Option<Task> {
        unsafe {
            self.with_inner(|inner| {
                inner
                    .map
                    .get_mut(key)
                    .map(|item| item.task.take().expect("Task has already been taken"))
            })
        }
    }

    pub fn reset(&self, key: TaskId, task: Task) {
        unsafe {
            self.with_inner(|inner| {
                let place = inner.map.get_mut(key).expect("Invalid key");
                debug_assert!(place.task.is_none(), "Task was not taken");
                place.task = Some(task);
            })
        }
    }

    pub fn insert<F: Future + 'static>(
        &self,
        shared: NonNull<Shared>,
        tracker: SendWrapper<()>,
        future: F,
    ) -> Task {
        unsafe {
            self.with_inner(|inner| {
                let mut ret = None;
                let key = inner.map.insert_with_key(|key| {
                    let [ptr, r] = Task::new::<F, 2>(key, shared, tracker, future);
                    ret = Some(r);
                    Item {
                        prev: None,
                        next: None,
                        task: Some(ptr),
                        is_hot: true,
                    }
                });
                inner.link_tail::<HOT>(key);
                ret.take().expect("Task was not initialized")
            })
        }
    }

    pub fn make_hot(&self, key: TaskId) {
        unsafe { self.with_inner(|inner| inner.make_hot(key)) }
    }

    pub fn make_cold(&self, key: TaskId) {
        unsafe { self.with_inner(|inner| inner.make_cold(key)) }
    }

    pub fn next_hot(&self, key: TaskId) -> Option<TaskId> {
        unsafe {
            self.with_inner(|inner| {
                inner.map.get(key).and_then(|item| {
                    debug_assert!(item.is_hot);
                    item.next
                })
            })
        }
    }

    pub fn hot_head(&self) -> Option<TaskId> {
        unsafe { self.with_inner(|inner| inner.hot.head) }
    }

    pub fn iter_hot(&self) -> Iter<'_> {
        Iter {
            queue: self,
            curr: self.hot_head(),
        }
    }

    pub fn remove(&self, key: TaskId) -> Option<Task> {
        unsafe {
            self.with_inner(|inner| {
                let is_hot = inner.map.get(key)?.is_hot;

                if is_hot {
                    inner.unlink::<HOT>(key);
                } else {
                    inner.unlink::<COLD>(key);
                };

                inner.map.remove(key)?.task
            })
        }
    }

    /// # Safety
    ///
    /// The caller must ensure that no concurrent access to the queue occurs
    /// while this reference is active.
    #[inline(always)]
    unsafe fn with_inner<R, F: FnOnce(&mut Inner) -> R>(&self, f: F) -> R {
        // SAFETY: Caller must ensure no concurrent access to the queue.
        self.inner.with_mut(|inner| f(unsafe { &mut *inner }))
    }
}

impl Inner {
    fn new(size: usize) -> Self {
        Self {
            map: SlotMap::with_capacity_and_key(size),
            hot: List::default(),
            cold: List::default(),
        }
    }

    /// Link a task to the end of a queue
    fn link_tail<const HOT: QueueMarker>(&mut self, key: TaskId) {
        let list = if HOT { &mut self.hot } else { &mut self.cold };
        let old_tail = list.tail;

        list.tail = Some(key);
        if list.head.is_none() {
            list.head = Some(key);
        }

        let item = self.map.get_mut(key).expect("item exists");
        item.prev = old_tail;
        item.next = None;
        item.is_hot = HOT;

        if let Some(tail_key) = old_tail {
            self.map.get_mut(tail_key).expect("tail exists").next = Some(key);
        }
    }

    fn unlink<const HOT: QueueMarker>(&mut self, key: TaskId) {
        let list = if HOT { &mut self.hot } else { &mut self.cold };

        let (prev, next) = {
            let item = self.map.get(key).expect("item exists");
            debug_assert_eq!(item.is_hot, HOT);
            (item.prev, item.next)
        };

        if list.head == Some(key) {
            list.head = next;
        }
        if list.tail == Some(key) {
            list.tail = prev;
        }

        if let Some(prev_key) = prev {
            self.map.get_mut(prev_key).expect("prev exists").next = next;
        }
        if let Some(next_key) = next {
            self.map.get_mut(next_key).expect("next exists").prev = prev;
        }
    }

    fn make_hot(&mut self, key: TaskId) {
        let Some(item) = self.map.get(key) else {
            return;
        };

        if item.is_hot {
            return;
        }

        self.unlink::<COLD>(key);
        self.link_tail::<HOT>(key);
    }

    fn make_cold(&mut self, key: TaskId) {
        let Some(item) = self.map.get(key) else {
            return;
        };

        debug_assert!(item.is_hot);

        self.unlink::<HOT>(key);
        self.link_tail::<COLD>(key);
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = TaskId;

    fn next(&mut self) -> Option<Self::Item> {
        let curr = self.curr?;
        self.curr = self.queue.next_hot(curr);
        Some(curr)
    }
}
