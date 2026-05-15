use compio::compat::{FuturesAdapter, RuntimeCompat};
use criterion::async_executor::AsyncExecutor;

pub struct CompioInFutures {
    runtime: RuntimeCompat<FuturesAdapter>,
}

impl Default for CompioInFutures {
    fn default() -> Self {
        let runtime =
            RuntimeCompat::<FuturesAdapter>::new(compio::runtime::Runtime::new().unwrap()).unwrap();
        Self { runtime }
    }
}

impl AsyncExecutor for CompioInFutures {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        (&self).block_on(future)
    }
}

impl AsyncExecutor for &CompioInFutures {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        futures_executor::block_on(self.runtime.execute(future))
    }
}
