use std::io;

use super::{fallback, iour};
use crate::buffer_pool::{BufPtr, Slot};

#[repr(transparent)]
#[derive(Debug)]
pub(in crate::sys) struct BufControl {
    inner: Inner,
}

#[derive(Debug)]
enum Inner {
    IoUring(iour::BufControl),
    Fallback(fallback::BufControl),
}

impl BufControl {
    pub unsafe fn new(
        driver: &mut crate::Driver,
        bufs: &[Slot],
        bufs_len: u32,
        flags: u16,
    ) -> io::Result<Self> {
        let inner = if driver.as_iour().is_some() {
            let ctrl = unsafe { iour::BufControl::new(driver, bufs, bufs_len, flags) }?;
            Inner::IoUring(ctrl)
        } else {
            Inner::Fallback(fallback::BufControl::new(bufs))
        };

        Ok(Self { inner })
    }

    pub unsafe fn release(&mut self, driver: &mut crate::Driver) -> io::Result<()> {
        match &mut self.inner {
            Inner::IoUring(control) => unsafe { control.release(driver) },
            Inner::Fallback(_) => Ok(()),
        }
    }

    /// Get the buffer group id
    pub fn buffer_group(&self) -> u16 {
        match &self.inner {
            Inner::IoUring(buf_control) => buf_control.buffer_group(),
            Inner::Fallback(_) => unreachable!("Buffer group is only used on io_uring"),
        }
    }

    pub unsafe fn reset(&mut self, buffer_id: u16, ptr: BufPtr, len: u32) {
        match &mut self.inner {
            Inner::IoUring(buf_control) => unsafe { buf_control.reset(buffer_id, ptr, len) },
            Inner::Fallback(buf_control) => unsafe { buf_control.reset(buffer_id, ptr, len) },
        }
    }

    pub fn pop(&mut self) -> io::Result<u16> {
        match &mut self.inner {
            Inner::IoUring(iour) => iour.pop(),
            Inner::Fallback(fallback) => fallback.pop(),
        }
    }

    pub fn is_io_uring(&self) -> bool {
        matches!(self.inner, Inner::IoUring(_))
    }
}
