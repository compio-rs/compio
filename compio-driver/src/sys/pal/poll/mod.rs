use std::{
    io::{IoSlice, IoSliceMut},
    slice,
};

use crate::{WaitArg, op::VectoredControl, sys::prelude::*};

mod_use![aio];

pub fn decide<E, F>(fd: RawFd, interest: Interest, f: F) -> io::Result<crate::Decision>
where
    F: FnMut() -> std::result::Result<usize, E>,
    E: Into<io::Error>,
{
    match poll_io(f)? {
        Poll::Ready(res) => Ok(crate::Decision::Completed(res)),
        Poll::Pending => Ok(crate::Decision::wait_for(fd, interest)),
    }
}

#[derive(Debug)]
pub(in crate::sys) struct Track {
    pub arg: WaitArg,
    pub ready: bool,
}

impl From<WaitArg> for Track {
    fn from(arg: WaitArg) -> Self {
        Self { arg, ready: false }
    }
}

impl VectoredControl {
    pub(crate) fn io_slices(&self) -> &[IoSlice<'_>] {
        // SAFETY: SysSlice is defined exactly the same as IoSlice
        unsafe { slice::from_raw_parts(self.slices.as_ptr().cast(), self.slices.len()) }
    }

    pub(crate) fn io_slices_mut(&mut self) -> &mut [IoSliceMut<'_>] {
        // SAFETY: SysSlice is defined exactly the same as IoSliceMut
        unsafe { slice::from_raw_parts_mut(self.slices.as_mut_ptr().cast(), self.slices.len()) }
    }
}
