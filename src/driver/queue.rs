#[cfg(not(feature = "sync-queue"))]
use std::{cell::UnsafeCell, collections::VecDeque};

#[cfg(feature = "sync-queue")]
use crossbeam_queue::SegQueue;

#[cfg(feature = "sync-queue")]
#[repr(transparent)]
pub(super) struct Queue<T>(SegQueue<T>);

#[cfg(feature = "sync-queue")]
impl<T> Queue<T> {
    pub fn with_capacity(_capacity: usize) -> Self {
        Self(SegQueue::new())
    }

    pub fn push(&self, value: T) {
        self.0.push(value);
    }

    pub fn pop(&self) -> Option<T> {
        self.0.pop()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(not(feature = "sync-queue"))]
#[repr(transparent)]
pub(super) struct Queue<T>(UnsafeCell<VecDeque<T>>);

#[cfg(not(feature = "sync-queue"))]
impl<T> Queue<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self(UnsafeCell::new(VecDeque::with_capacity(capacity)))
    }

    pub fn push(&self, value: T) {
        unsafe {
            // SAFETY: we expect that only single thread uses the queue
            (&mut *self.0.get()).push_back(value);
        }
    }

    pub fn pop(&self) -> Option<T> {
        unsafe {
            // SAFETY: we expect that only single thread uses the queue
            (&mut *self.0.get()).pop_front()
        }
    }

    pub fn len(&self) -> usize {
        unsafe {
            // SAFETY: we expect that only single thread uses the queue
            (&*self.0.get()).len()
        }
    }

    pub fn is_empty(&self) -> bool {
        unsafe {
            // SAFETY: we expect that only single thread uses the queue
            (&*self.0.get()).is_empty()
        }
    }
}
