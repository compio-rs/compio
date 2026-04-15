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

use crate::sys::{extra::StubExtra, prelude::*};

/// Operations.
pub trait OpCode {
    type Control: Default;

    unsafe fn init(&mut self, ctrl: &mut Self::Control);

    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
    }
}

pub(crate) trait Carry {
    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra);
}

impl<T: OpCode> Carry for Carrier<T> {
    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra) {}
}

#[derive(Debug)]
pub(crate) struct Driver(PhantomData<ErasedKey>);

impl Driver {
    pub fn new(_: &ProactorBuilder) -> io::Result<Self> {
        Ok(Self(PhantomData))
    }

    pub fn driver_type(&self) -> DriverType {
        stub_unimpl()
    }

    pub fn attach(&mut self, _: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, _: ErasedKey) {}

    pub(in crate::sys) fn default_extra(&self) -> StubExtra {
        StubExtra::new()
    }

    pub fn push(&mut self, _: ErasedKey) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(stub_error()))
    }

    pub fn poll(&mut self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }

    pub fn waker(&self) -> Waker {
        futures_util::task::noop_waker()
    }

    pub fn create_buffer_pool(&mut self, _: u16, _: usize) -> io::Result<BufferPool> {
        Err(stub_error())
    }

    pub unsafe fn release_buffer_pool(&mut self, _: BufferPool) -> io::Result<()> {
        Ok(())
    }

    pub fn pop_multishot(&mut self, _: &ErasedKey) -> Option<BufResult<usize, crate::sys::Extra>> {
        None
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        stub_unimpl()
    }
}
