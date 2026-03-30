use std::{io, mem::MaybeUninit};

use super::iour;
use crate::sys::{
    buffer_pool::{Slot, fallback},
    fusion::FuseDriver,
};

#[repr(transparent)]
#[derive(Debug)]
pub struct BufControl {
    inner: Inner,
}

#[derive(Debug)]
enum Inner {
    IoUring(iour::BufControl),
    Fallback(fallback::BufControl),
}

impl BufControl {
    pub unsafe fn new(driver: &mut super::Driver, bufs: &[Slot], flags: u16) -> io::Result<Self> {
        let inner = match &mut driver.fuse {
            FuseDriver::IoUring(driver) => {
                let ctrl = unsafe { iour::BufControl::new(driver, bufs, flags) }?;
                Inner::IoUring(ctrl)
            }
            FuseDriver::Poll(_) => Inner::Fallback(fallback::BufControl::new(bufs)),
        };
        Ok(Self { inner })
    }

    pub unsafe fn release(&mut self, driver: &mut super::Driver) -> io::Result<()> {
        match &mut self.inner {
            Inner::IoUring(control) => unsafe {
                control.release(driver.as_iour_mut().expect("Should be io_uring"))
            },
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

    pub unsafe fn reset(&mut self, buffer_id: u16, buf: &[MaybeUninit<u8>]) {
        match &mut self.inner {
            Inner::IoUring(buf_control) => unsafe { buf_control.reset(buffer_id, buf) },
            Inner::Fallback(buf_control) => unsafe { buf_control.reset(buffer_id, buf) },
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
