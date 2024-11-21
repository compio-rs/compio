use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode, PushEntry};

use crate::runtime::Runtime;

#[derive(Debug)]
pub struct OpFlagsFuture<T: OpCode> {
    key: Option<Key<T>>,
}

impl<T: OpCode> OpFlagsFuture<T> {
    pub fn new(key: Key<T>) -> Self {
        Self { key: Some(key) }
    }
}

impl<T: OpCode> Future for OpFlagsFuture<T> {
    type Output = (BufResult<usize, T>, u32);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = Runtime::with_current(|r| r.poll_task(cx, self.key.take().unwrap()));
        match res {
            PushEntry::Pending(key) => {
                self.key = Some(key);
                Poll::Pending
            }
            PushEntry::Ready(res) => Poll::Ready(res),
        }
    }
}

impl<T: OpCode> Drop for OpFlagsFuture<T> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            // If there's no runtime, it's OK to forget it.
            Runtime::try_with_current(|r| r.cancel_op(key)).ok();
        }
    }
}
