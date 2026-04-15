use io_uring::{opcode, types::Fd};

use crate::{
    IourOpCode as OpCode, OpEntry,
    sys::{op::*, prelude::*},
};

/// Accept multiple connections.
pub struct AcceptMulti<S> {
    pub(crate) op: Accept<S>,
    multishots: VecDeque<AcceptMultishotResult>,
}

impl<S> AcceptMulti<S> {
    /// Create [`AcceptMulti`].
    pub fn new(fd: S) -> Self {
        Self {
            op: Accept::new(fd),
            multishots: VecDeque::new(),
        }
    }
}

struct AcceptMultishotResult {
    res: io::Result<Socket2>,
    extra: crate::Extra,
}

impl AcceptMultishotResult {
    pub unsafe fn new(res: io::Result<usize>, extra: crate::Extra) -> Self {
        Self {
            res: res.map(|fd| unsafe { Socket2::from_raw_fd(fd as _) }),
            extra,
        }
    }

    pub fn into_result(self) -> BufResult<usize, crate::Extra> {
        BufResult(self.res.map(|fd| fd.into_raw_fd() as _), self.extra)
    }
}

unsafe impl<S: AsFd> OpCode for AcceptMulti<S> {
    type Control = <Accept<S> as OpCode>::Control;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::AcceptMulti::new(Fd(self.op.fd.as_fd().as_raw_fd()))
            .flags(libc::SOCK_CLOEXEC)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, res, extra) }
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.multishots
            .push_back(unsafe { AcceptMultishotResult::new(res, extra) });
    }

    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.multishots.pop_front().map(|res| res.into_result())
    }
}

impl<S> IntoInner for AcceptMulti<S> {
    type Inner = Socket2;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner().0
    }
}
