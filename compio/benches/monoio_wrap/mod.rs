use std::{cell::RefCell, future::Future};

use criterion::async_executor::AsyncExecutor;

pub struct MonoioRuntime(RefCell<monoio::Runtime<monoio::IoUringDriver>>);

impl Default for MonoioRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MonoioRuntime {
    pub fn new() -> Self {
        Self(RefCell::new(
            monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
                .build()
                .unwrap(),
        ))
    }
}

impl AsyncExecutor for MonoioRuntime {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.borrow_mut().block_on(future)
    }
}

impl AsyncExecutor for &MonoioRuntime {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.borrow_mut().block_on(future)
    }
}
