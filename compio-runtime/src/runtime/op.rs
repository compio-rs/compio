use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode};

use crate::runtime::Runtime;

#[derive(Debug)]
pub struct OpFuture<T> {
    user_data: Key<T>,
    completed: bool,
}

impl<T> OpFuture<T> {
    pub fn new(user_data: Key<T>) -> Self {
        Self {
            user_data,
            completed: false,
        }
    }
}

impl<T: OpCode> Future for OpFuture<T> {
    type Output = BufResult<usize, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = Runtime::current().inner().poll_task(cx, self.user_data);
        if res.is_ready() {
            self.completed = true;
        }
        res
    }
}

impl<T> Drop for OpFuture<T> {
    fn drop(&mut self) {
        if !self.completed {
            Runtime::current().inner().cancel_op(self.user_data)
        }
    }
}
