use super::*;
/// Abstraction of IOCP operations.
///
/// # Safety
///
/// Implementors must ensure that the operation is safe to be polled
/// according to the returned [`OpType`].
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
    unsafe fn init(&mut self, ctrl: &mut Self::Control);

    /// Determines that the operation is really overlapped defined by
    /// Windows API. If not, the driver will try to operate it in
    /// another thread.
    fn op_type(&self, control: &Self::Control) -> OpType {
        _ = control;
        OpType::Overlapped
    }

    /// Perform Windows API call with given pointer to overlapped struct.
    ///
    /// It is always safe to cast `optr` to a pointer to
    /// [`Overlapped<Self>`].
    ///
    /// Don't do heavy work here if [`OpCode::op_type`] returns
    /// [`OpType::Event`].
    ///
    /// # Safety
    ///
    /// * `self` must be alive until the operation completes.
    /// * When [`OpCode::op_type`] returns [`OpType::Blocking`], this method is
    ///   called in another thread.
    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>>;

    /// Cancel the async IO operation.
    ///
    /// Usually it calls `CancelIoEx`.
    // # Safety for implementors
    //
    // `optr` must not be dereferenced. It's only used as a marker to identify the
    // operation.
    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        _ = control;
        _ = optr;
        Ok(())
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
}

pub(crate) trait Carry {
    fn op_type(&self) -> OpType;

    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;

    fn cancel(&mut self, optr: *mut OVERLAPPED) -> io::Result<()>;

    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra);
}

impl<T: OpCode> Carry for Carrier<T> {
    fn op_type(&self) -> OpType {
        let (op, control) = self.as_iocp();
        op.op_type(control)
    }

    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let (op, control) = self.as_iocp_mut();
        unsafe { op.operate(control, optr) }
    }

    fn cancel(&mut self, optr: *mut OVERLAPPED) -> io::Result<()> {
        let (op, control) = self.as_iocp_mut();
        op.cancel(control, optr)
    }

    unsafe fn set_result(&mut self, res: &io::Result<usize>, extra: &crate::Extra) {
        let (op, control) = self.as_iocp_mut();
        unsafe { op.set_result(control, res, extra) }
    }
}
