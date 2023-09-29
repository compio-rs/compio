//! This mod doesn't actually contain any driver, but meant to provide some
//! common op type and utilities for unix platform (for iour and polling).

pub(crate) mod op;

use std::{mem::ManuallyDrop, pin::Pin, ptr::NonNull};

use crate::OpCode;

pub struct RawOp(NonNull<dyn OpCode>);

impl RawOp {
    pub fn new(_user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Box::new(op);
        Self(unsafe { NonNull::new_unchecked(Box::into_raw(op as Box<dyn OpCode>)) })
    }

    pub fn as_pin(&mut self) -> Pin<&mut dyn OpCode> {
        unsafe { Pin::new_unchecked(self.0.as_mut()) }
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
