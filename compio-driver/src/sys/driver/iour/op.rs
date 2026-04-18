use std::io;

pub use OpCode as IourOpCode;
use compio_buf::BufResult;

use crate::control::Carrier;

/// The created entry of [`OpCode`].
pub enum OpEntry {
    /// This operation creates an io-uring submission entry.
    Submission(io_uring::squeue::Entry),
    #[cfg(feature = "io-uring-sqe128")]
    /// This operation creates an 128-bit io-uring submission entry.
    Submission128(io_uring::squeue::Entry128),
    /// This operation is a blocking one.
    Blocking,
}

/// Abstraction of io-uring operations.
///
/// # Safety
///
/// The returned Entry from `create_entry` must be valid until the operation
/// is completed.
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

    /// Create submission entry.
    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry;

    /// Create submission entry for fallback. This method will only be
    /// called if `create_entry` returns an entry with unsupported
    /// opcode.
    fn create_entry_fallback(&mut self, _: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    /// Call the operation in a blocking way. This method will be called if
    /// * [`create_entry`] returns [`OpEntry::Blocking`].
    /// * [`create_entry`] returns an entry with unsupported opcode, and
    ///   [`create_entry_fallback`] returns [`OpEntry::Blocking`].
    /// * [`create_entry`] and [`create_entry_fallback`] both return an entry
    ///   with unsupported opcode.
    ///
    /// [`create_entry`]: OpCode::create_entry
    /// [`create_entry_fallback`]: OpCode::create_entry_fallback
    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        unreachable!("this operation is asynchronous")
    }

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

    /// Push a multishot result to the inner queue.
    ///
    /// # Safety
    ///
    /// The params must be the result coming from this operation.
    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        _: io::Result<usize>,
        _: crate::Extra,
    ) {
        unreachable!("this operation is not multishot")
    }

    /// Pop a multishot result from the inner queue.
    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        unreachable!("this operation is not multishot")
    }
}

impl OpEntry {
    pub(crate) fn with_extra(self, extra: &crate::Extra) -> Self {
        let Some(extra) = extra.try_as_iour() else {
            return self;
        };
        match self {
            Self::Submission(mut entry) => Self::Submission({
                if let Some(personality) = extra.get_personality() {
                    entry = entry.personality(personality);
                }
                // Set the union of two flags - it will not remove previous flags set by the Op
                entry.flags(extra.get_sqe_flags())
            }),
            #[cfg(feature = "io-uring-sqe128")]
            Self::Submission128(mut entry) => Self::Submission128({
                if let Some(personality) = extra.get_personality() {
                    entry = entry.personality(personality);
                }
                entry.flags(extra.get_sqe_flags())
            }),
            Self::Blocking => Self::Blocking,
        }
    }
}

pub(crate) trait Carry {
    /// See [`OpCode::create_entry`].
    fn create_entry(&mut self) -> OpEntry;

    /// See [`OpCode::create_entry_fallback`].
    fn create_entry_fallback(&mut self) -> OpEntry;

    /// See [`OpCode::call_blocking`].
    fn call_blocking(&mut self) -> io::Result<usize>;

    /// See [`OpCode::set_result`].
    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra);

    /// See [`OpCode::push_multishot`].
    unsafe fn push_multishot(&mut self, _: io::Result<usize>, _: crate::Extra);

    /// See [`OpCode::pop_multishot`].
    fn pop_multishot(&mut self) -> Option<BufResult<usize, crate::sys::Extra>>;
}

impl<T: crate::OpCode> Carry for Carrier<T> {
    fn create_entry(&mut self) -> OpEntry {
        let (op, control) = self.as_iour();
        op.create_entry(control)
    }

    fn create_entry_fallback(&mut self) -> OpEntry {
        let (op, control) = self.as_iour();
        op.create_entry_fallback(control)
    }

    fn call_blocking(&mut self) -> io::Result<usize> {
        let (op, control) = self.as_iour();
        op.call_blocking(control)
    }

    unsafe fn set_result(&mut self, result: &io::Result<usize>, extra: &crate::Extra) {
        let (op, control) = self.as_iour();
        unsafe { OpCode::set_result(op, control, result, extra) }
    }

    unsafe fn push_multishot(&mut self, result: io::Result<usize>, extra: crate::Extra) {
        let (op, control) = self.as_iour();
        unsafe { op.push_multishot(control, result, extra) }
    }

    fn pop_multishot(&mut self) -> Option<BufResult<usize, crate::sys::Extra>> {
        let (op, control) = self.as_iour();
        op.pop_multishot(control)
    }
}

impl From<io_uring::squeue::Entry> for OpEntry {
    fn from(value: io_uring::squeue::Entry) -> Self {
        Self::Submission(value)
    }
}

#[cfg(feature = "io-uring-sqe128")]
impl From<io_uring::squeue::Entry128> for OpEntry {
    fn from(value: io_uring::squeue::Entry128) -> Self {
        Self::Submission128(value)
    }
}
