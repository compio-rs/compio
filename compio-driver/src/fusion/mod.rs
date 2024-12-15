#[path = "../poll/mod.rs"]
mod poll;

#[path = "../iour/mod.rs"]
mod iour;

pub(crate) mod op;

#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
pub use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::{io, task::Poll, time::Duration};

pub use iour::{OpCode as IourOpCode, OpEntry};
pub(crate) use iour::{sockaddr_storage, socklen_t};
pub use poll::{Decision, OpCode as PollOpCode, OpType};

pub use crate::driver_type::DriverType; // Re-export so current user won't be broken
use crate::{Key, ProactorBuilder};

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
        match DriverType::current() {
            DriverType::Poll => Ok(Self {
                fuse: FuseDriver::Poll(poll::Driver::new(builder)?),
            }),
            DriverType::IoUring => Ok(Self {
                fuse: FuseDriver::IoUring(iour::Driver::new(builder)?),
            }),
            _ => unreachable!("Fuse driver will only be enabled on linux"),
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

    pub fn handle(&self) -> NotifyHandle {
        let fuse = match &self.fuse {
            FuseDriver::Poll(driver) => FuseNotifyHandle::Poll(driver.handle()),
            FuseDriver::IoUring(driver) => FuseNotifyHandle::IoUring(driver.handle()),
        };
        NotifyHandle::from_fuse(fuse)
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

enum FuseNotifyHandle {
    Poll(poll::NotifyHandle),
    IoUring(iour::NotifyHandle),
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    fuse: FuseNotifyHandle,
}

impl NotifyHandle {
    fn from_fuse(fuse: FuseNotifyHandle) -> Self {
        Self { fuse }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        match &self.fuse {
            FuseNotifyHandle::Poll(handle) => handle.notify(),
            FuseNotifyHandle::IoUring(handle) => handle.notify(),
        }
    }
}
