pub use self::OpCode as PollOpCode;
use crate::sys::prelude::*;

/// Represents the filter type of kqueue. `polling` crate doesn't expose such
/// API, and we need to know about it when `cancel` is called.
#[non_exhaustive]
pub enum OpType {
    /// The operation polls an fd.
    Fd(Multi<RawFd>),
    /// The operation submits an AIO.
    #[cfg(aio)]
    Aio(NonNull<libc::aiocb>),
}

/// Result of [`OpCode::pre_submit`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(Multi<WaitArg>),
    /// Blocking operation, needs to be spawned in another thread
    Blocking,
    /// AIO operation, needs to be spawned to the kernel.
    #[cfg(aio)]
    Aio(AioArg),
}

/// Meta of polling operations.
#[derive(Debug, Clone, Copy)]
pub struct WaitArg {
    /// The raw fd of the operation.
    pub fd: RawFd,
    /// The interest to be registered.
    pub interest: Interest,
}

/// Abstraction of operations.
///
/// # Safety
///
/// If `pre_submit` returns `Decision::Wait`, `op_type` must also return
/// `Some(OpType::Fd)` with same fds as the `WaitArg`s. Similarly, if
/// `pre_submit` returns `Decision::Aio`, `op_type` must return
/// `Some(OpType::Aio)` with the correct `aiocb` pointer.
pub unsafe trait OpCode {
    /// Type that contains self-references and other needed info during the
    /// operation
    type Control: Default;

    /// Initialize the control
    ///
    /// # Safety
    ///
    /// Caller must guarantee that during the lifetime of `ctrl`, `Self` is
    /// unmoved and valid.
    unsafe fn init(&mut self, _: &mut Self::Control) {}

    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to polling is required.
    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision>;

    /// Get the operation type when an event is occurred.
    fn op_type(&mut self, _: &mut Self::Control) -> Option<OpType> {
        None
    }

    /// Perform the operation after received corresponding
    /// event. If this operation is blocking, the return value should be
    /// [`Poll::Ready`].
    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>>;

    /// Set the result when it completes.
    /// The operation stores the result and is responsible to release it if
    /// the operation is cancelled.
    ///
    /// # Safety
    ///
    /// The params must be the result coming from this operation.
    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
    }
}

pub(crate) trait Carry {
    fn pre_submit(&mut self) -> io::Result<Decision>;
    fn op_type(&mut self) -> Option<OpType>;
    fn operate(&mut self) -> Poll<io::Result<usize>>;
    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra);
}

impl OpType {
    /// Create an [`OpType::Fd`] with one [`RawFd`].
    pub fn fd(fd: RawFd) -> Self {
        Self::Fd(Multi::from_buf([fd]))
    }

    /// Create an [`OpType::Fd`] with multiple [`RawFd`]s.
    pub fn multi_fd<I: IntoIterator<Item = RawFd>>(fds: I) -> Self {
        Self::Fd(Multi::from_iter(fds))
    }
}

impl<T: crate::OpCode> Carry for Carrier<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        let (op, control) = self.as_poll();
        op.pre_submit(control)
    }

    fn op_type(&mut self) -> Option<OpType> {
        let (op, control) = self.as_poll();
        op.op_type(control)
    }

    fn operate(&mut self) -> Poll<io::Result<usize>> {
        let (op, control) = self.as_poll();
        op.operate(control)
    }

    unsafe fn set_result(&mut self, res: &io::Result<usize>, extra: &crate::Extra) {
        let (op, control) = self.as_poll();
        unsafe { OpCode::set_result(op, control, res, extra) }
    }
}

impl Decision {
    /// Decide to wait for the given fd with the given interest.
    pub fn wait_for(fd: RawFd, interest: Interest) -> Self {
        Self::Wait(Multi::from_buf([WaitArg { fd, interest }]))
    }

    /// Decide to wait for many fds.
    pub fn wait_for_many<I: IntoIterator<Item = WaitArg>>(args: I) -> Self {
        Self::Wait(Multi::from_iter(args))
    }

    /// Decide to wait for the given fd to be readable.
    pub fn wait_readable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::Readable)
    }

    /// Decide to wait for the given fd to be writable.
    pub fn wait_writable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::Writable)
    }

    /// Decide to spawn an AIO operation. `submit` is a method like
    /// `aio_read`.
    #[cfg(aio)]
    pub fn aio(
        cb: &mut libc::aiocb,
        submit: unsafe extern "C" fn(*mut libc::aiocb) -> i32,
    ) -> Self {
        Self::Aio(AioArg {
            aiocbp: NonNull::from(cb),
            submit,
        })
    }
}

impl WaitArg {
    /// Create a new readable `WaitArg`.
    pub fn readable(fd: RawFd) -> Self {
        Self {
            fd,
            interest: Interest::Readable,
        }
    }

    /// Create a new writable `WaitArg`.
    pub fn writable(fd: RawFd) -> Self {
        Self {
            fd,
            interest: Interest::Writable,
        }
    }
}
