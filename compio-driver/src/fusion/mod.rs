#[path = "../poll/mod.rs"]
mod poll;

#[path = "../iour/mod.rs"]
mod iour;

pub(crate) mod op;

#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
pub use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::{io, task::Poll, time::Duration};

pub use driver_type::DriverType;
pub use iour::{OpCode as IourOpCode, OpEntry};
pub use poll::{Decision, OpCode as PollOpCode};

use crate::{Key, OutEntries, ProactorBuilder};

mod driver_type {
    use std::sync::atomic::{AtomicU8, Ordering};

    const UNINIT: u8 = u8::MAX;
    const IO_URING: u8 = 0;
    const POLLING: u8 = 1;

    static DRIVER_TYPE: AtomicU8 = AtomicU8::new(UNINIT);

    /// Representing underlying driver type the fusion driver is using
    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DriverType {
        /// Using `polling` driver
        Poll    = POLLING,

        /// Using `io-uring` driver
        IoUring = IO_URING,
    }

    impl DriverType {
        fn from_num(n: u8) -> Self {
            match n {
                IO_URING => Self::IoUring,
                POLLING => Self::Poll,
                _ => unreachable!("invalid driver type"),
            }
        }

        /// Get the underlying driver type
        pub fn current() -> DriverType {
            match DRIVER_TYPE.load(Ordering::Acquire) {
                UNINIT => {}
                x => return DriverType::from_num(x),
            }

            let dev_ty = if uring_available() {
                DriverType::IoUring
            } else {
                DriverType::Poll
            };

            DRIVER_TYPE.store(dev_ty as u8, Ordering::Release);

            dev_ty
        }
    }

    fn uring_available() -> bool {
        use io_uring::opcode::*;

        // Add more opcodes here if used
        const USED_OP: &[u8] = &[
            Read::CODE,
            Readv::CODE,
            Write::CODE,
            Writev::CODE,
            Fsync::CODE,
            Accept::CODE,
            Connect::CODE,
            RecvMsg::CODE,
            SendMsg::CODE,
            AsyncCancel::CODE,
            OpenAt::CODE,
            Close::CODE,
            Shutdown::CODE,
            // Linux kernel 5.19
            #[cfg(any(
                feature = "io-uring-sqe128",
                feature = "io-uring-cqe32",
                feature = "io-uring-socket"
            ))]
            Socket::CODE,
        ];

        Ok(())
            .and_then(|_| {
                let uring = io_uring::IoUring::new(2)?;
                let mut probe = io_uring::Probe::new();
                uring.submitter().register_probe(&mut probe)?;
                std::io::Result::Ok(USED_OP.iter().all(|op| probe.is_supported(*op)))
            })
            .unwrap_or(false)
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
        match DriverType::current() {
            DriverType::Poll => Ok(Self {
                fuse: FuseDriver::Poll(poll::Driver::new(builder)?),
            }),
            DriverType::IoUring => Ok(Self {
                fuse: FuseDriver::IoUring(iour::Driver::new(builder)?),
            }),
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

    pub fn cancel<T>(&mut self, op: Key<T>) {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.cancel(op),
            FuseDriver::IoUring(driver) => driver.cancel(op),
        }
    }

    pub fn push<T: OpCode + 'static>(&mut self, op: &mut Key<T>) -> Poll<io::Result<usize>> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.push(op),
            FuseDriver::IoUring(driver) => driver.push(op),
        }
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: OutEntries<impl Extend<usize>>,
    ) -> io::Result<()> {
        match &mut self.fuse {
            FuseDriver::Poll(driver) => driver.poll(timeout, entries),
            FuseDriver::IoUring(driver) => driver.poll(timeout, entries),
        }
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        let fuse = match &self.fuse {
            FuseDriver::Poll(driver) => FuseNotifyHandle::Poll(driver.handle()?),
            FuseDriver::IoUring(driver) => FuseNotifyHandle::IoUring(driver.handle()?),
        };
        Ok(NotifyHandle::from_fuse(fuse))
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
