//! This mod doesn't actually contain any driver, but meant to provide some
//! common op type and utilities for unix platform (for iour and polling).

pub(crate) mod op;

use std::{mem::ManuallyDrop, pin::Pin, ptr::NonNull};

use crate::OpCode;

pub(crate) struct RawOp {
    op: NonNull<dyn OpCode>,
    // The two flags here are manual reference counting. The driver holds the strong ref until it
    // completes; the runtime holds the strong ref until the future is dropped.
    completed: bool,
    cancelled: bool,
}

impl RawOp {
    pub(crate) fn new(_user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Box::new(op);
        Self {
            op: unsafe { NonNull::new_unchecked(Box::into_raw(op as Box<dyn OpCode>)) },
            completed: false,
            cancelled: false,
        }
    }

    pub fn as_pin(&mut self) -> Pin<&mut dyn OpCode> {
        unsafe { Pin::new_unchecked(self.op.as_mut()) }
    }

    pub fn set_completed(&mut self) -> bool {
        self.completed = true;
        self.cancelled
    }

    pub fn set_cancelled(&mut self) -> bool {
        self.cancelled = true;
        self.completed
    }

    /// # Safety
    /// The caller should ensure the correct type.
    pub unsafe fn into_inner<T: OpCode>(self) -> T {
        let this = ManuallyDrop::new(self);
        *Box::from_raw(this.op.cast().as_ptr())
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        if self.completed {
            let _ = unsafe { Box::from_raw(self.op.as_ptr()) };
        }
    }
}
