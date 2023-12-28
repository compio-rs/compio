//! This mod doesn't actually contain any driver, but meant to provide some
//! common op type and utilities for unix platform (for iour and polling).

pub(crate) mod op;

use std::{io, mem::ManuallyDrop, pin::Pin, ptr::NonNull};

use compio_buf::BufResult;

use crate::OpCode;

pub(crate) struct RawOp {
    op: NonNull<dyn OpCode>,
    // The two flags here are manual reference counting. The driver holds the strong ref until it
    // completes; the runtime holds the strong ref until the future is dropped.
    cancelled: bool,
    result: Option<io::Result<usize>>,
}

impl RawOp {
    pub(crate) fn new(_user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Box::new(op);
        Self {
            op: unsafe { NonNull::new_unchecked(Box::into_raw(op as Box<dyn OpCode>)) },
            cancelled: false,
            result: None,
        }
    }

    pub fn as_pin(&mut self) -> Pin<&mut dyn OpCode> {
        unsafe { Pin::new_unchecked(self.op.as_mut()) }
    }

    pub fn set_cancelled(&mut self) -> bool {
        self.cancelled = true;
        self.has_result()
    }

    pub fn set_result(&mut self, res: io::Result<usize>) -> bool {
        self.result = Some(res);
        self.cancelled
    }

    pub fn has_result(&self) -> bool {
        self.result.is_some()
    }

    /// # Safety
    /// The caller should ensure the correct type.
    pub unsafe fn into_inner<T: OpCode>(self) -> BufResult<usize, T> {
        let mut this = ManuallyDrop::new(self);
        let op = *Box::from_raw(this.op.cast().as_ptr());
        BufResult(this.result.take().unwrap(), op)
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        if self.has_result() {
            let _ = unsafe { Box::from_raw(self.op.as_ptr()) };
        }
    }
}
