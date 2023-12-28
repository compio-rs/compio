use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode};

use crate::Runtime;

#[derive(Default)]
pub(crate) struct OpRuntime {
    ops: HashMap<usize, Option<Waker>>,
}

impl OpRuntime {
    pub fn update_waker(&mut self, key: usize, waker: Waker) {
        *self.ops.entry(key).or_default() = Some(waker)
    }

    pub fn wake(&mut self, key: usize) {
        if let Some(waker) = self.ops.entry(key).or_default() {
            waker.wake_by_ref();
        }
    }

    pub fn remove(&mut self, key: usize) {
        self.ops.remove(&key).unwrap();
    }
}

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

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = Runtime::current().inner().poll_task(cx, self.user_data);
        if res.is_ready() {
            self.get_mut().completed = true;
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
