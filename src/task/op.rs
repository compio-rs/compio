use std::{
    future::Future,
    io,
    marker::PhantomData,
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use slab::Slab;

use crate::driver::OpCode;

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

pub struct RegisteredOp {
    pub op: Option<RawOp>,
    pub waker: Option<Waker>,
    pub result: Option<io::Result<usize>>,
    pub cancelled: bool,
}

impl RegisteredOp {
    fn new(op: Option<RawOp>) -> Self {
        Self {
            op,
            waker: None,
            result: None,
            cancelled: false,
        }
    }
}

#[derive(Default)]
pub struct OpRuntime {
    ops: Slab<RegisteredOp>,
}

impl OpRuntime {
    pub fn insert<T: OpCode + 'static>(&mut self, op: T) -> (usize, &mut RawOp) {
        let user_data = self.ops.insert(RegisteredOp::new(Some(RawOp::new(op))));
        let op = unsafe { self.ops.get_unchecked_mut(user_data) };
        (user_data, op.op.as_mut().unwrap())
    }

    pub fn insert_dummy(&mut self) -> usize {
        self.ops.insert(RegisteredOp::new(None))
    }

    pub fn update_waker(&mut self, key: usize, waker: Waker) {
        if let Some(op) = self.ops.get_mut(key) {
            op.waker = Some(waker);
        }
    }

    pub fn update_result(&mut self, key: usize, result: io::Result<usize>) {
        let op = self.ops.get_mut(key).unwrap();
        if let Some(waker) = op.waker.take() {
            waker.wake();
        }
        op.result = Some(result);
        if op.cancelled {
            self.remove(key);
        }
    }

    pub fn has_result(&mut self, key: usize) -> bool {
        self.ops.get_mut(key).unwrap().result.is_some()
    }

    pub fn cancel(&mut self, key: usize) {
        self.ops.get_mut(key).unwrap().cancelled = true;
    }

    pub fn remove(&mut self, key: usize) -> RegisteredOp {
        self.ops.remove(key)
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
