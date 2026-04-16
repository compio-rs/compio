use std::{fmt::Debug, io};

use crate::buffer_pool::*;

cfg_if::cfg_if! {
    if #[cfg(fusion)] {
        mod fusion;
        mod iour;
        mod fallback;
        use fusion as imp;
    } else if #[cfg(io_uring)] {
        mod iour;
        use iour as imp;
    } else {
        mod fallback;
        use fallback as imp;
    }
}

#[doc(hidden)]
pub struct BufControl(imp::BufControl);

impl Debug for BufControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl BufControl {
    pub(crate) unsafe fn new(
        driver: &mut super::Driver,
        bufs: &[Slot],
        buf_len: u32,
        flags: u16,
    ) -> io::Result<BufControl> {
        #[cfg(io_uring)]
        let inner = unsafe { imp::BufControl::new(driver, bufs, buf_len, flags)? };

        #[cfg(not(io_uring))]
        let inner = fallback::BufControl::new(bufs);

        _ = (driver, buf_len, flags);

        Ok(Self(inner))
    }

    pub unsafe fn release(&mut self, driver: &mut crate::Driver) -> io::Result<()> {
        unsafe { self.0.release(driver) }
    }

    pub fn pop(&mut self) -> io::Result<u16> {
        self.0.pop()
    }

    pub unsafe fn reset(&mut self, buffer_id: u16, ptr: BufPtr, len: u32) {
        unsafe { self.0.reset(buffer_id, ptr, len) }
    }

    /// Get the buffer group id
    #[cfg(io_uring)]
    pub fn buffer_group(&self) -> u16 {
        self.0.buffer_group()
    }

    /// Test if the buffer pool is an io_uring one.
    #[cfg(fusion)]
    pub fn is_io_uring(&self) -> bool {
        self.0.is_io_uring()
    }
}
