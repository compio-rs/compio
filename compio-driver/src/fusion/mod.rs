#[path = "../poll/mod.rs"]
mod poll;

#[path = "../iour/mod.rs"]
mod iour;

pub(crate) mod op;

#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::{
    io,
    task::{Poll, Waker},
    time::Duration,
};

use compio_log::warn;
pub(crate) use iour::is_op_supported;
pub use iour::{OpCode as IourOpCode, OpEntry};
pub use poll::{Decision, OpCode as PollOpCode, OpType};

pub use crate::driver_type::DriverType; // Re-export so current user won't be broken
use crate::{BufferPool, Key, ProactorBuilder};

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

    pub fn driver_type(&self) -> DriverType {
        match &self.fuse {
            FuseDriver::Poll(driver) => driver.driver_type(),
            FuseDriver::IoUring(driver) => driver.driver_type(),
        }
    }

    pub fn create_op<T: OpCode + 'static>(&self, op: T) -> Key<T> {
        match &self.fuse {
            FuseDriver::Poll(driver) => driver.create_op(op),
            FuseDriver::IoUring(driver) => driver.create_op(op),
        }
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.attach(fd),
            FuseDriver::IoUring(driver) => driver.attach(fd),
        }
    }

    pub fn cancel(&mut self, op: &mut Key<dyn OpCode>) {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.cancel(op),
            FuseDriver::IoUring(driver) => driver.cancel(op),
        }
    }

    pub fn push(&mut self, op: &mut Key<dyn OpCode>) -> Poll<io::Result<usize>> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.push(op),
            FuseDriver::IoUring(driver) => driver.push(op),
        }
    }

    pub unsafe fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
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
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.release_buffer_pool(buffer_pool),
            FuseDriver::IoUring(driver) => driver.release_buffer_pool(buffer_pool),
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
