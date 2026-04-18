use windows_sys::Win32::System::IO::OVERLAPPED;

use crate::{OpCode, sys::op::*};

unsafe impl<S: AsFd> OpCode for ReadManagedAt<S> {
    type Control = ();

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<S: AsFd> OpCode for ReadManaged<S> {
    type Control = ();

    unsafe fn operate(
        &mut self,
        control: &mut (),
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<S: AsFd> OpCode for RecvManaged<S> {
    type Control = RecvControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromManaged<S> {
    type Control = RecvFromControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromMulti<S> {
    type Control = RecvFromControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
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

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, res, extra) }
    }
}

unsafe impl<S: AsFd> OpCode for RecvMsgMulti<S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        unsafe { self.op.operate(control, optr) }
    }

    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        self.op.cancel(control, optr)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, res, extra) };
        if let Ok(res) = res {
            self.len = *res;
        }
    }
}
