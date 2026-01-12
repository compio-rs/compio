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
use std::{
    io,
    task::{Poll, Waker},
    time::Duration,
};

use crate::{BufferPool, DriverType, ErasedKey, Key, ProactorBuilder};

pub struct Extra {}

impl Extra {
    pub fn new() -> Self {
        Self {}
    }
}

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
    pub fn new(_: &ProactorBuilder) -> io::Result<Self> {
        Err(stub_error())
    }

    pub fn driver_type(&self) -> DriverType {
        stub_unimpl()
    }

    pub fn attach(&mut self, _: RawFd) -> io::Result<()> {
        Err(stub_error())
    }

    pub fn cancel<T: OpCode>(&mut self, _: Key<T>) {
        stub_unimpl()
    }

    pub fn default_extra(&self) -> Extra {
        Extra::new()
    }

    pub fn push(&mut self, _: ErasedKey) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(stub_error()))
    }

    pub fn poll(&mut self, _: Option<Duration>) -> io::Result<()> {
        Err(stub_error())
    }

    pub fn waker(&self) -> Waker {
        futures_util::task::noop_waker()
    }

    pub fn create_buffer_pool(&mut self, _: u16, _: usize) -> io::Result<BufferPool> {
        Err(stub_error())
    }

    pub unsafe fn release_buffer_pool(&mut self, _: BufferPool) -> io::Result<()> {
        Err(stub_error())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        stub_unimpl()
    }
}
