use std::{
    collections::HashMap,
    future::Future,
    io,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use crate::{
    driver::{OpCode, RawOp},
    key::Key,
};

#[derive(Default)]
pub struct RegisteredOp {
    pub op: Option<RawOp>,
    pub waker: Option<Waker>,
    pub result: Option<io::Result<usize>>,
    pub cancelled: bool,
}

#[derive(Default)]
pub(crate) struct OpRuntime {
    ops: HashMap<usize, RegisteredOp>,
}

impl OpRuntime {
    pub fn update_waker<T>(&mut self, key: Key<T>, waker: Waker) {
        self.ops.entry(*key).or_default().waker = Some(waker);
    }

    pub fn update_result<T>(&mut self, key: Key<T>, raw_op: RawOp, result: io::Result<usize>) {
        let op = self.ops.entry(*key).or_default();
        if let Some(waker) = op.waker.take() {
            waker.wake();
        }
        op.op = Some(raw_op);
        op.result = Some(result);
        if op.cancelled {
            self.remove(key);
        }
    }

    pub fn has_result<T>(&mut self, key: Key<T>) -> bool {
        self.ops
            .get_mut(&key)
            .map(|op| op.result.is_some())
            .unwrap_or_default()
    }

    pub fn cancel<T>(&mut self, key: Key<T>) {
        self.ops.entry(*key).or_default().cancelled = true;
    }

    pub fn remove<T>(&mut self, key: Key<T>) -> RegisteredOp {
        self.ops.remove(&key).unwrap()
    }
}

#[derive(Debug)]
pub struct OpFuture<T: 'static> {
    user_data: Key<T>,
    completed: bool,
    _p: PhantomData<&'static T>,
}

impl<T> OpFuture<T> {
    pub fn new(user_data: Key<T>) -> Self {
        Self {
            user_data,
            completed: false,
            _p: PhantomData,
        }
    }
}

impl<T: OpCode> Future for OpFuture<T> {
    type Output = (io::Result<usize>, T);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = crate::task::RUNTIME.with(|runtime| runtime.poll_task(cx, self.user_data));
        if res.is_ready() {
            self.get_mut().completed = true;
        }
        res
    }
}

impl<T> Drop for OpFuture<T> {
    fn drop(&mut self) {
        if !self.completed {
            crate::task::RUNTIME.with(|runtime| runtime.cancel_op(self.user_data))
        }
    }
}
