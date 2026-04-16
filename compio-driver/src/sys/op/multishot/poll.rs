use crate::{
    Decision, OpType, PollOpCode as OpCode,
    sys::{op::*, prelude::*},
};

/// Accept multiple connections.
pub struct AcceptMulti<S> {
    pub(crate) op: Accept<S>,
}

impl<S> AcceptMulti<S> {
    /// Create [`AcceptMulti`].
    pub fn new(fd: S) -> Self {
        Self {
            op: Accept::new(fd),
        }
    }
}

impl<S> IntoInner for AcceptMulti<S> {
    type Inner = Socket2;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner().0
    }
}

unsafe impl<S: AsFd> OpCode for AcceptMulti<S> {
    type Control = <Accept<S> as OpCode>::Control;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}
