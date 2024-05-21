use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode, PushEntry};

use crate::runtime::Runtime;

#[derive(Debug)]
pub struct OpFuture<T: OpCode> {
    key: Option<Key<T>>,
}

impl<T: OpCode> OpFuture<T> {
    pub fn new(key: Key<T>) -> Self {
        Self { key: Some(key) }
    }
}

impl<T: OpCode> Future for OpFuture<T> {
    type Output = BufResult<usize, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = Runtime::current()
            .inner()
            .poll_task(cx, self.key.take().unwrap());
        match res {
            PushEntry::Pending(key) => {
                self.key = Some(key);
                Poll::Pending
            }
            PushEntry::Ready(res) => Poll::Ready(res),
        }
    }
}

impl<T: OpCode> Drop for OpFuture<T> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            Runtime::current().inner().cancel_op(key)
        }
    }
}
