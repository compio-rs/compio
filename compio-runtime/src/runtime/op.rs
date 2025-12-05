use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Extra, Key, OpCode, PushEntry};
use futures_util::future::FusedFuture;

use crate::runtime::Runtime;

#[derive(Debug)]
pub struct OpFuture<T: OpCode, E> {
    key: Option<Key<T>>,
    _e: std::marker::PhantomData<E>,
}

impl<T: OpCode> OpFuture<T, ()> {
    pub fn new(key: Key<T>) -> Self {
        Self {
            key: Some(key),
            _e: std::marker::PhantomData,
        }
    }
}

impl<T: OpCode> OpFuture<T, Extra> {
    pub fn new_extra(key: Key<T>) -> Self {
        Self {
            key: Some(key),
            _e: std::marker::PhantomData,
        }
    }
}

impl<T: OpCode> Future for OpFuture<T, ()> {
    type Output = BufResult<usize, T>;

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

impl<T: OpCode> Future for OpFuture<T, Extra> {
    type Output = (BufResult<usize, T>, Extra);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = Runtime::with_current(|r| r.poll_task_with_extra(cx, self.key.take().unwrap()));
        match res {
            PushEntry::Pending(key) => {
                self.key = Some(key);
                Poll::Pending
            }
            PushEntry::Ready(res) => Poll::Ready(res),
        }
    }
}

impl<T: OpCode, E> FusedFuture for OpFuture<T, E>
where
    OpFuture<T, E>: Future,
{
    fn is_terminated(&self) -> bool {
        self.key.is_none()
    }
}

impl<T: OpCode, E> Drop for OpFuture<T, E> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            Runtime::with_current(|r| r.cancel_op(key));
        }
    }
}
