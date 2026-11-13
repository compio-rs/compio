use windows_sys::Win32::System::IO::OVERLAPPED;

use crate::{AsSocketState, OpCode, sys::op::*};

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

    unsafe fn completion_error(
        &self,
        control: &Self::Control,
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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

    unsafe fn completion_error(
        &self,
        control: &(),
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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

    unsafe fn completion_error(
        &self,
        control: &Self::Control,
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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

    unsafe fn completion_error(
        &self,
        control: &Self::Control,
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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

    unsafe fn completion_error(
        &self,
        control: &Self::Control,
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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

unsafe impl<C: IoBufMut, S: AsSocketState> OpCode for RecvMsgManaged<C, S> {
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

    unsafe fn completion_error(
        &self,
        control: &Self::Control,
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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

unsafe impl<S: AsSocketState> OpCode for RecvMsgMulti<S> {
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

    unsafe fn completion_error(
        &self,
        control: &Self::Control,
        optr: *const OVERLAPPED,
        error: io::Error,
    ) -> io::Error {
        unsafe { self.op.completion_error(control, optr, error) }
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
