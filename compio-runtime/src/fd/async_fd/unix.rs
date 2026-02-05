use std::os::fd::{FromRawFd, RawFd};

use compio_driver::AsFd;

use crate::fd::AsyncFd;

impl<T: AsFd + FromRawFd> FromRawFd for AsyncFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        unsafe { Self::new_unchecked(FromRawFd::from_raw_fd(fd)) }
    }
}
