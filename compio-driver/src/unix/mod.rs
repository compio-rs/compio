//! This mod doesn't actually contain any driver, but meant to provide some
//! common op type and utilities for unix platform (for iour and polling).

pub(crate) mod op;

use std::{io, mem::ManuallyDrop, pin::Pin, ptr::NonNull, task::Waker};

use compio_buf::BufResult;

use crate::{OpCode, PushEntry};

#[repr(transparent)]
pub(crate) struct Overlapped<T: ?Sized> {
    pub op: T,
}

impl<T> Overlapped<T> {
    pub(crate) fn new(_driver: RawFd, _user_data: usize, op: T) -> Self {
        Self { op }
    }
}
