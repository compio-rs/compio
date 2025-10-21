use std::{cell::UnsafeCell, collections::VecDeque};

/// A queue that is `!Sync` with interior mutability.
pub(crate) struct LocalQueue<T> {
    queue: UnsafeCell<VecDeque<T>>,
}

impl<T> LocalQueue<T> {
    /// Creates an empty `LocalQueue`.
    pub(crate) const fn new() -> Self {
        Self {
            queue: UnsafeCell::new(VecDeque::new()),
        }
    }

    /// Pushes an item to the back of the queue.
    pub(crate) fn push(&self, item: T) {
        // SAFETY:
        // Exclusive mutable access because:
        // - The mutable reference is created and used immediately within this scope.
        // - `LocalQueue` is `!Sync`, so no other threads can access it concurrently.
        let queue = unsafe { &mut *self.queue.get() };
        queue.push_back(item);
    }

    /// Pops an item from the front of the queue, returning `None` if empty.
    pub(crate) fn pop(&self) -> Option<T> {
        // SAFETY:
        // Exclusive mutable access because:
        // - The mutable reference is created and used immediately within this scope.
        // - `LocalQueue` is `!Sync`, so no other threads can access it concurrently.
        let queue = unsafe { &mut *self.queue.get() };
        queue.pop_front()
    }

    /// Returns `true` if the queue is empty.
    pub(crate) fn is_empty(&self) -> bool {
        // SAFETY:
        // Exclusive mutable access because:
        // - The mutable reference is created and used immediately within this scope.
        // - `LocalQueue` is `!Sync`, so no other threads can access it concurrently.
        let queue = unsafe { &mut *self.queue.get() };
        queue.is_empty()
    }
}
