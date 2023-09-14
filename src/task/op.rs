use std::{
    future::Future,
    io,
    marker::PhantomData,
    mem::ManuallyDrop,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll, Waker},
};

use slab::Slab;

use crate::{driver::OpCode, key::Key};

pub struct RawOp(NonNull<dyn OpCode>);

impl RawOp {
    pub fn new(op: impl OpCode + 'static) -> Self {
        let op = Box::new(op);
        Self(unsafe { NonNull::new_unchecked(Box::into_raw(op as Box<dyn OpCode>)) })
    }

    pub fn as_dyn_mut(&mut self) -> &mut dyn OpCode {
        unsafe { self.0.as_mut() }
    }

    pub unsafe fn into_inner<T: OpCode>(self) -> T {
        let this = ManuallyDrop::new(self);
        *Box::from_raw(this.0.cast().as_ptr())
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        drop(unsafe { Box::from_raw(self.0.as_ptr()) })
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
pub(crate) struct OpRuntime {
    ops: Slab<RegisteredOp>,
}

impl OpRuntime {
    pub fn insert<T: OpCode + 'static>(&mut self, op: T) -> Key<T> {
        let user_data = self.ops.insert(RegisteredOp::new(Some(RawOp::new(op))));
        // Safety: `user_data` corresponds to `op` inserted which has type `T`.
        unsafe { Key::new(user_data) }
    }

    pub fn insert_dummy(&mut self) -> Key<()> {
        Key::new_dummy(self.ops.insert(RegisteredOp::new(None)))
    }

    pub fn get_raw_op(&mut self, key: usize) -> &mut RawOp {
        self.ops.get_mut(key).unwrap().op.as_mut().unwrap()
    }

    pub fn update_waker<T>(&mut self, key: Key<T>, waker: Waker) {
        if let Some(op) = self.ops.get_mut(*key) {
            op.waker = Some(waker);
        }
    }

    pub fn update_result<T>(&mut self, key: Key<T>, result: io::Result<usize>) {
        if let Some(op) = self.ops.get_mut(*key) {
            if let Some(waker) = op.waker.take() {
                waker.wake();
            }
            op.result = Some(result);
            if op.cancelled {
                self.remove(key);
            }
        }
    }

    pub fn has_result<T>(&mut self, key: Key<T>) -> bool {
        self.ops
            .get_mut(*key)
            .map(|op| op.result.is_some())
            .unwrap_or_default()
    }

    pub fn cancel<T>(&mut self, key: Key<T>) {
        if let Some(ops) = self.ops.get_mut(*key) {
            ops.cancelled = true;
        }
    }

    pub fn remove<T>(&mut self, key: Key<T>) -> RegisteredOp {
        self.ops.remove(*key)
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

impl Future for OpFuture<()> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = crate::task::RUNTIME.with(|runtime| runtime.poll_dummy(cx, self.user_data));
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
