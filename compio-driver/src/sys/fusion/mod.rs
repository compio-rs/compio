pub(crate) mod op;

#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::{
    io,
    task::{Poll, Waker},
    time::Duration,
};

use compio_log::warn;
pub use iour::{IourOpCode, OpEntry};
pub use poll::{Decision, OpType, PollOpCode};

pub(crate) use super::iour::is_op_supported;
use super::{iour, poll};
pub use crate::driver_type::DriverType; // Re-export so current user won't be broken
use crate::{BufferPool, ProactorBuilder, key::ErasedKey};

pub enum Extra {
    Poll(poll::Extra),
    IoUring(iour::Extra),
}

impl From<poll::Extra> for Extra {
    fn from(inner: poll::Extra) -> Self {
        Self::Poll(inner)
    }
}

impl From<iour::Extra> for Extra {
    fn from(inner: iour::Extra) -> Self {
        Self::IoUring(inner)
    }
}

#[allow(dead_code)]
impl super::Extra {
    pub(crate) fn is_iour(&self) -> bool {
        matches!(self.0, Extra::IoUring(_))
    }

    pub(crate) fn is_poll(&self) -> bool {
        matches!(self.0, Extra::Poll(_))
    }

    pub(in crate::sys) fn try_as_iour(&self) -> Option<&iour::Extra> {
        if let Extra::IoUring(extra) = &self.0 {
            Some(extra)
        } else {
            None
        }
    }

    pub(in crate::sys) fn try_as_iour_mut(&mut self) -> Option<&mut iour::Extra> {
        if let Extra::IoUring(extra) = &mut self.0 {
            Some(extra)
        } else {
            None
        }
    }

    pub(in crate::sys) fn try_as_poll(&self) -> Option<&poll::Extra> {
        if let Extra::Poll(extra) = &self.0 {
            Some(extra)
        } else {
            None
        }
    }

    pub(in crate::sys) fn try_as_poll_mut(&mut self) -> Option<&mut poll::Extra> {
        if let Extra::Poll(extra) = &mut self.0 {
            Some(extra)
        } else {
            None
        }
    }
}

/// Fused [`OpCode`]
///
/// This trait encapsulates both operation for `io-uring` and `polling`
pub trait OpCode: PollOpCode + IourOpCode {}

impl<T: PollOpCode + IourOpCode + ?Sized> OpCode for T {}

#[allow(clippy::large_enum_variant)]
enum FuseDriver {
    Poll(poll::Driver),
    IoUring(iour::Driver),
}

/// Low-level fusion driver.
pub(crate) struct Driver {
    fuse: FuseDriver,
}

impl Driver {
    /// Create a new fusion driver with given number of entries
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        let (ty, fallback) = match &builder.driver_type {
            Some(t) => (*t, false),
            None => (DriverType::suggest(), true),
        };
        match ty {
            DriverType::Poll => Ok(Self {
                fuse: FuseDriver::Poll(poll::Driver::new(builder)?),
            }),
            DriverType::IoUring => match iour::Driver::new(builder) {
                Ok(driver) => Ok(Self {
                    fuse: FuseDriver::IoUring(driver),
                }),
                // use _e here so that when `enable_log` is disabled, clippy won't complain
                Err(_e) if fallback => {
                    warn!("Fail to create io-uring driver: {_e:?}, fallback to polling driver.");
                    Ok(Self {
                        fuse: FuseDriver::Poll(poll::Driver::new(builder)?),
                    })
                }
                Err(e) => Err(e),
            },
            _ => unreachable!("Fuse driver will only be enabled on linux"),
        }
    }

    #[allow(dead_code)]
    pub fn as_iour(&self) -> Option<&iour::Driver> {
        if let FuseDriver::IoUring(driver) = &self.fuse {
            Some(driver)
        } else {
            None
        }
    }

    pub fn driver_type(&self) -> DriverType {
        match &self.fuse {
            FuseDriver::Poll(driver) => driver.driver_type(),
            FuseDriver::IoUring(driver) => driver.driver_type(),
        }
    }

    pub fn default_extra(&self) -> Extra {
        match &self.fuse {
            FuseDriver::Poll(driver) => Extra::Poll(driver.default_extra()),
            FuseDriver::IoUring(driver) => Extra::IoUring(driver.default_extra()),
        }
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.attach(fd),
            FuseDriver::IoUring(driver) => driver.attach(fd),
        }
    }

    pub fn cancel(&mut self, key: ErasedKey) {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.cancel(key),
            FuseDriver::IoUring(driver) => driver.cancel(key),
        }
    }

    pub fn push(&mut self, op: ErasedKey) -> Poll<io::Result<usize>> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.push(op),
            FuseDriver::IoUring(driver) => driver.push(op),
        }
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.poll(timeout),
            FuseDriver::IoUring(driver) => driver.poll(timeout),
        }
    }

    pub fn waker(&self) -> Waker {
        match &self.fuse {
            FuseDriver::Poll(driver) => driver.waker(),
            FuseDriver::IoUring(driver) => driver.waker(),
        }
    }

    pub fn create_buffer_pool(
        &mut self,
        buffer_len: u16,
        buffer_size: usize,
    ) -> io::Result<BufferPool> {
        match &mut self.fuse {
            FuseDriver::IoUring(driver) => Ok(driver.create_buffer_pool(buffer_len, buffer_size)?),
            FuseDriver::Poll(driver) => Ok(driver.create_buffer_pool(buffer_len, buffer_size)?),
        }
    }

    /// # Safety
    ///
    /// caller must make sure release the buffer pool with correct driver
    pub unsafe fn release_buffer_pool(&mut self, buffer_pool: BufferPool) -> io::Result<()> {
        unsafe {
            match &mut self.fuse {
                FuseDriver::Poll(driver) => driver.release_buffer_pool(buffer_pool),
                FuseDriver::IoUring(driver) => driver.release_buffer_pool(buffer_pool),
            }
        }
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        match &self.fuse {
            FuseDriver::Poll(driver) => driver.as_raw_fd(),
            FuseDriver::IoUring(driver) => driver.as_raw_fd(),
        }
    }
}
