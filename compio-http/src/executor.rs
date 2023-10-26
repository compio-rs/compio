use std::{future::Future, pin::Pin};

use hyper::rt::Executor;

#[derive(Debug, Clone)]
pub struct CompioExecutor;

impl Executor<Pin<Box<dyn Future<Output = ()> + Send>>> for CompioExecutor {
    fn execute(&self, fut: Pin<Box<dyn Future<Output = ()> + Send>>) {
        compio_runtime::spawn(fut).detach()
    }
}
