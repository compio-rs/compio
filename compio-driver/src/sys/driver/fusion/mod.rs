use std::time::Duration;

pub use iour::{IourOpCode, OpEntry};
pub use poll::{Decision, OpType, PollOpCode, WaitArg};

use super::{iour, poll};
use crate::sys::{extra::FuseExtra, prelude::*};

mod op;
pub use op::*;

/// Low-level fusion driver.
pub(crate) struct Driver {
    pub fuse: FuseDriver,
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum FuseDriver {
    Poll(poll::Driver),
    IoUring(iour::Driver),
}

impl Driver {
    /// Create a new fusion driver with given number of entries
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        let (ty, fallback) = match &builder.driver_type {
            Some(t) => (*t, false),
            None => (DriverType::suggest(builder.op_flags), true),
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

    #[allow(dead_code)]
    pub fn as_iour_mut(&mut self) -> Option<&mut iour::Driver> {
        if let FuseDriver::IoUring(driver) = &mut self.fuse {
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

    pub(in crate::sys) fn default_extra(&self) -> FuseExtra {
        match &self.fuse {
            FuseDriver::Poll(driver) => FuseExtra::Poll(driver.default_extra()),
            FuseDriver::IoUring(driver) => FuseExtra::IoUring(driver.default_extra()),
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

    pub fn pop_multishot(
        &mut self,
        key: &ErasedKey,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.pop_multishot(key),
            FuseDriver::IoUring(driver) => driver.pop_multishot(key),
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
