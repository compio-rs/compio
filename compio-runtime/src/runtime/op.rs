use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use compio_buf::BufResult;
use compio_driver::{Entry, OpCode};

use crate::{key::Key, Runtime};

#[derive(Default)]
pub(crate) struct RegisteredOp {
    pub waker: Option<Waker>,
    pub entry: Option<Entry>,
    pub cancelled: bool,
}

#[derive(Default)]
pub(crate) struct OpRuntime {
    ops: HashMap<usize, RegisteredOp>,
}

impl OpRuntime {
    pub fn update_waker(&mut self, key: usize, waker: Waker) {
        self.ops.entry(key).or_default().waker = Some(waker);
    }

    pub fn update_result(&mut self, key: usize, entry: Entry) {
        let op = self.ops.entry(key).or_default();
        if let Some(waker) = op.waker.take() {
            waker.wake();
        }
        op.entry = Some(entry);
        if op.cancelled {
            self.remove(key);
        }
    }

    pub fn has_result(&mut self, key: usize) -> bool {
        self.ops
            .get_mut(&key)
            .map(|op| op.entry.is_some())
            .unwrap_or_default()
    }

    pub fn cancel(&mut self, key: usize) {
        self.ops.entry(key).or_default().cancelled = true;
    }

    pub fn remove(&mut self, key: usize) -> RegisteredOp {
        self.ops.remove(&key).unwrap()
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
