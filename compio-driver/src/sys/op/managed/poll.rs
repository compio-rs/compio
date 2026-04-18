use super::fallback::*;
use crate::{Decision, OpType, PollOpCode as OpCode, op::RecvMsgControl, sys::prelude::*};

unsafe impl<S: AsFd> OpCode for ReadManaged<S> {
    type Control = ();

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}

unsafe impl<S: AsFd> OpCode for ReadManagedAt<S> {
    type Control = AioControl;

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

unsafe impl<S: AsFd> OpCode for RecvManaged<S> {
    type Control = ();

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromManaged<S> {
    type Control = ();

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

unsafe impl<S: AsFd> OpCode for RecvFromMulti<S> {
    type Control = ();

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }

    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        result: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        if let Ok(result) = result {
            self.len = *result;
        }
    }
}

unsafe impl<C: IoBufMut, S: AsFd> OpCode for RecvMsgManaged<C, S> {
    type Control = RecvMsgControl;

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

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, result, extra) }
    }
}

unsafe impl<S: AsFd> OpCode for RecvMsgMulti<S> {
    type Control = RecvMsgControl;

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

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, result, extra) };
        if let Ok(result) = result {
            self.len = *result;
        }
    }
}
