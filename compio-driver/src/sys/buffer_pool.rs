use std::{fmt::Debug, io};

use crate::buffer_pool::*;

cfg_if::cfg_if! {
    if #[cfg(io_uring)] {
        use super::imp;
    } else {
        use fallback as imp;
    }
}

pub(crate) struct BufControl(imp::BufControl);

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

#[cfg(any(fusion, not(io_uring)))]
pub(in crate::sys) mod fallback {
    use std::{collections::VecDeque, io};

    use super::*;
    use crate::buffer_pool::BufPtr;

    #[derive(Debug)]
    pub(in crate::sys) struct BufControl {
        queue: VecDeque<u16>,
    }

    impl BufControl {
        pub fn new(bufs: &[Slot]) -> Self {
            assert!(bufs.len() < u16::MAX as _);
            Self {
                queue: bufs.iter().enumerate().map(|(id, _)| id as u16).collect(),
            }
        }

        #[allow(dead_code)]
        pub unsafe fn release(&mut self, _: &mut crate::Driver) -> io::Result<()> {
            Ok(())
        }

        pub fn pop(&mut self) -> io::Result<u16> {
            self.queue
                .pop_front()
                .ok_or_else(|| io::Error::other("buffer ring has no available buffer"))
        }

        pub unsafe fn reset(&mut self, buffer_id: u16, _: BufPtr, _: u32) {
            self.queue.push_back(buffer_id);
        }
    }
}
