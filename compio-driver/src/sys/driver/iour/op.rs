use std::io;

pub use OpCode as IourOpCode;
use compio_buf::BufResult;
use compio_log::trace;

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

fn push_se(
    mut entry: super::SEntry,
    extra: &crate::Extra,
    user_data: u64,
    sq: &mut io_uring::squeue::SubmissionQueue<'_, super::SEntry>,
) -> Pushed {
    if let Some(extra) = extra.try_as_iour() {
        if let Some(personality) = extra.get_personality() {
            entry = entry.personality(personality);
        }
        // Set the union of two flags - it will not remove previous flags set by the Op
        entry = entry.flags(extra.get_sqe_flags());
    }
    let entry = entry.user_data(user_data);
    // SAFETY: the op lives in the `Key` until its completion is reaped.
    match unsafe { sq.push(&entry) } {
        Ok(()) => Pushed::Done,
        Err(_) => Pushed::Full(entry),
    }
}

/// Push `entry` into `sq` if the kernel supports its opcode, otherwise call
/// `unsupported`.
fn push_op_entry(
    entry: OpEntry,
    extra: &crate::Extra,
    user_data: u64,
    sq: &mut io_uring::squeue::SubmissionQueue<'_, super::SEntry>,
    unsupported: impl FnOnce(&mut io_uring::squeue::SubmissionQueue<'_, super::SEntry>) -> Pushed,
) -> Pushed {
    let entry: super::SEntry = match entry {
        #[allow(clippy::useless_conversion)]
        OpEntry::Submission(e) => e.into(),
        #[cfg(feature = "io-uring-sqe128")]
        OpEntry::Submission128(e) => e,
        OpEntry::Blocking => return Pushed::Blocking,
    };
    if crate::sys::is_op_supported(entry.get_opcode() as _) {
        push_se(entry, extra, user_data, sq)
    } else {
        unsupported(sq)
    }
}

/// The opcode from [`OpCode::create_entry`] is unsupported by the kernel;
/// retry with [`OpCode::create_entry_fallback`].
#[cold]
#[inline(never)]
fn push_fallback<T: crate::OpCode>(
    op: &mut T,
    control: &mut <T as OpCode>::Control,
    extra: &crate::Extra,
    user_data: u64,
    sq: &mut io_uring::squeue::SubmissionQueue<'_, super::SEntry>,
) -> Pushed {
    trace!(fallback = true, "push entry");
    let entry = op.create_entry_fallback(control);
    push_op_entry(entry, extra, user_data, sq, |_| Pushed::Blocking)
}

/// Outcome of [`Carry::push_into`].
pub(crate) enum Pushed {
    /// The entry is in the submission queue.
    Done,
    /// The submission queue is full; the ready entry is handed back.
    Full(super::SEntry),
    /// The op wants to run on the blocking pool.
    Blocking,
}

pub(crate) trait Carry {
    /// Create the entry (falling back to [`OpCode::create_entry_fallback`] if
    /// the opcode is unsupported), apply `extra` and `user_data`, and push it
    /// into `sq`.
    fn push_into(
        &mut self,
        extra: &crate::Extra,
        user_data: u64,
        sq: &mut io_uring::squeue::SubmissionQueue<'_, super::SEntry>,
    ) -> Pushed;

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
    fn push_into(
        &mut self,
        extra: &crate::Extra,
        user_data: u64,
        sq: &mut io_uring::squeue::SubmissionQueue<'_, super::SEntry>,
    ) -> Pushed {
        let (op, control) = self.as_iour();
        trace!(fallback = false, "push entry");
        let entry = op.create_entry(control);
        push_op_entry(entry, extra, user_data, sq, |sq| {
            push_fallback(op, control, extra, user_data, sq)
        })
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
