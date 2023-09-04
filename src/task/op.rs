use crate::driver::OpCode;
use slab::Slab;
use std::{
    collections::HashMap,
    future::Future,
    io,
    marker::PhantomData,
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll, Waker},
};

pub struct RawOp(*mut dyn OpCode);

impl RawOp {
    pub fn new(op: impl OpCode + 'static) -> Self {
        let op = Box::new(op);
        Self(Box::into_raw(op as Box<dyn OpCode>))
    }

    pub unsafe fn as_mut<T: OpCode>(&mut self) -> &mut T {
        &mut *(self.0 as *mut T)
    }

    pub unsafe fn into_inner<T: OpCode>(self) -> T {
        let this = ManuallyDrop::new(self);
        *Box::from_raw(this.0 as *mut T)
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        drop(unsafe { Box::from_raw(self.0) })
    }
}

#[derive(Default)]
pub struct OpRuntime {
    ops: Slab<Option<RawOp>>,
    wakers: HashMap<usize, Waker>,
}

impl OpRuntime {
    pub fn insert<T: OpCode + 'static>(&mut self, op: T) -> (usize, &mut RawOp) {
        let user_data = self.ops.insert(Some(RawOp::new(op)));
        let op = unsafe { self.ops.get_unchecked_mut(user_data) };
        (user_data, op.as_mut().unwrap())
    }

    pub fn insert_dummy(&mut self) -> usize {
        self.ops.insert(None)
    }

    pub fn update_waker(&mut self, user_data: usize, waker: Waker) {
        self.wakers.insert(user_data, waker);
    }

    pub fn cancel(&mut self, user_data: usize) {
        self.wakers.remove(&user_data);
    }

    pub fn remove(&mut self, user_data: usize) -> (Option<RawOp>, Option<Waker>) {
        if self.ops.contains(user_data) {
            let op = self.ops.remove(user_data);
            let waker = self.wakers.remove(&user_data);
            (op, waker)
        } else {
            (None, None)
        }
    }
}

#[derive(Debug)]
pub struct OpFuture<T: OpCode + 'static> {
    user_data: usize,
    completed: bool,
    _p: PhantomData<&'static T>,
}

impl<T: OpCode> OpFuture<T> {
    pub fn new(user_data: usize) -> Self {
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

impl<T: OpCode> Drop for OpFuture<T> {
    fn drop(&mut self) {
        if !self.completed {
            crate::task::RUNTIME.with(|runtime| runtime.cancel_op(self.user_data))
        }
    }
}
