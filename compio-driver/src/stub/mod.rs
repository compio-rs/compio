#[cfg(not(feature = "proc_macro_diagnostic"))]
macro_rules! compile_warning {
    ($expr:expr) => {
        #[warn(dead_code)]
        const WARNING: &str = $expr;
    };
}
#[cfg(feature = "proc_macro_diagnostic")]
use compile_warning::compile_warning;

compile_warning!("You have to choose at least one of these features: [\"io-uring\", \"polling\"]");

#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::{io, task::Poll, time::Duration};

use crate::{BufferPool, DriverType, Key, ProactorBuilder};

/// Operations.
pub trait OpCode {}

pub mod op;

fn stub_error() -> io::Error {
    io::Error::other("Stub driver does not support any operations")
}

fn stub_unimpl() -> ! {
    unimplemented!("Stub driver does not support any operations")
}

#[derive(Debug)]
pub(crate) struct Driver(());

impl Driver {
    pub fn new(_builder: &ProactorBuilder) -> io::Result<Self> {
        Err(stub_error())
    }

    pub fn driver_type(&self) -> DriverType {
        stub_unimpl()
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Err(stub_error())
    }

    pub fn cancel(&mut self, _op: &mut Key<dyn OpCode>) {
        stub_unimpl()
    }

    pub fn create_op<T: OpCode + 'static>(&self, _op: T) -> Key<T> {
        stub_unimpl()
    }

    pub fn push(&mut self, _op: &mut Key<dyn crate::sys::OpCode>) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(stub_error()))
    }

    pub unsafe fn poll(&mut self, _timeout: Option<Duration>) -> io::Result<()> {
        Err(stub_error())
    }

    pub fn handle(&self) -> NotifyHandle {
        stub_unimpl()
    }

    pub fn create_buffer_pool(
        &mut self,
        _buffer_len: u16,
        _buffer_size: usize,
    ) -> io::Result<BufferPool> {
        Err(stub_error())
    }

    pub unsafe fn release_buffer_pool(&mut self, _buffer_pool: BufferPool) -> io::Result<()> {
        Err(stub_error())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        stub_unimpl()
    }
}

/// A notify handle to the inner driver.
#[derive(Debug, Clone)]
pub struct NotifyHandle(());

impl NotifyHandle {
    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        Err(stub_error())
    }
}
