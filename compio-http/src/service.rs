use std::future::Future;

use hyper::rt::Executor;

/// An executor service based on [`compio_runtime`]. It uses
/// [`compio_runtime::spawn`] interally.
#[derive(Debug, Default, Clone)]
pub struct CompioExecutor;

impl<F: Future<Output = ()> + Send + 'static> Executor<F> for CompioExecutor {
    fn execute(&self, fut: F) {
        compio_runtime::spawn(fut).detach();
    }
}
