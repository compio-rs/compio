use std::{cell::UnsafeCell, collections::VecDeque};

#[repr(transparent)]
pub struct Queue<T>(UnsafeCell<VecDeque<T>>);

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

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        unsafe {
            // SAFETY: we expect that only single thread uses the queue
            (&*self.0.get()).len()
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        unsafe {
            // SAFETY: we expect that only single thread uses the queue
            (&*self.0.get()).is_empty()
        }
    }
}
