//! This mod doesn't actually contain any driver, but meant to provide some
//! common op type and utilities for unix platform (for iour and polling).

pub(crate) mod op;

use crate::RawFd;

/// The overlapped struct for unix needn't contain extra fields.
#[repr(transparent)]
pub(crate) struct Overlapped<T: ?Sized> {
    pub op: T,
}

impl<T> Overlapped<T> {
    pub(crate) fn new(_driver: RawFd, _user_data: usize, op: T) -> Self {
        Self { op }
    }
}
