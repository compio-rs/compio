use compio::compat::{RuntimeCompat, TokioAdapter};
use criterion::async_executor::AsyncExecutor;

pub struct CompioInTokio {
    truntime: tokio::runtime::Runtime,
    cruntime: RuntimeCompat<TokioAdapter>,
}

impl Default for CompioInTokio {
    fn default() -> Self {
        let truntime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = truntime.enter();
        let cruntime =
            RuntimeCompat::<TokioAdapter>::new(compio::runtime::Runtime::new().unwrap()).unwrap();
        Self { truntime, cruntime }
    }
}

impl AsyncExecutor for CompioInTokio {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        (&self).block_on(future)
    }
}

impl AsyncExecutor for &CompioInTokio {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.truntime.block_on(self.cruntime.execute(future))
    }
}
