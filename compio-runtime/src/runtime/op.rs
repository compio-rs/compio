use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode};

use crate::runtime::{FutureState, Runtime};

#[derive(Default)]
pub(crate) struct OpRuntime {
    ops: HashMap<usize, FutureState>,
}

impl OpRuntime {
    pub fn update_waker(&mut self, key: usize, waker: Waker) {
        *self.ops.entry(key).or_default() = FutureState::Active(Some(waker));
    }

    pub fn wake(&mut self, key: usize) {
        let state = self.ops.entry(key).or_default();
        let old_state = std::mem::replace(state, FutureState::Completed);
        if let FutureState::Active(Some(waker)) = old_state {
            waker.wake();
        }
    }

    // Returns whether the op is completed.
    pub fn cancel(&mut self, key: usize) -> bool {
        let state = self.ops.remove(&key);
        state
            .map(|state| matches!(state, FutureState::Completed))
            .unwrap_or(true)
    }
}

#[derive(Debug)]
pub struct OpFuture<T> {
    user_data: Key<T>,
}

impl<T> OpFuture<T> {
    pub fn new(user_data: Key<T>) -> Self {
        Self { user_data }
    }
}

impl<T: OpCode> Future for OpFuture<T> {
    type Output = BufResult<usize, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Runtime::current().inner().poll_task(cx, self.user_data)
    }
}

impl<T> Drop for OpFuture<T> {
    fn drop(&mut self) {
        Runtime::current().inner().cancel_op(self.user_data)
    }
}
