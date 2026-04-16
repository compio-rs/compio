use io_uring::{opcode, types::Fd};

use crate::{IourOpCode as OpCode, OpEntry, op::*, sys::prelude::*};

/// Zerocopy [`Send`].
pub struct SendZc<T: IoBuf, S> {
    pub(crate) op: Send<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

/// Zerocopy [`SendTo`].
pub struct SendToZc<T: IoBuf, S: AsFd> {
    pub(crate) op: SendTo<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

/// Zerocopy [`SendVectored`].
pub struct SendVectoredZc<T: IoVectoredBuf, S> {
    pub(crate) op: SendVectored<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

/// Zerocopy [`SendToVectored`].
pub struct SendToVectoredZc<T: IoVectoredBuf, S: AsFd> {
    pub(crate) op: SendToVectored<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

/// Zerocopy [`SendMsg`].
pub struct SendMsgZc<T: IoVectoredBuf, C: IoBuf, S> {
    pub(crate) op: SendMsg<T, C, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

impl<T: IoBuf, S> SendZc<T, S> {
    /// Create [`SendZc`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            op: Send::new(fd, buffer, flags),
            res: None,
        }
    }
}

impl<T: IoBuf, S> IntoInner for SendZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendZc<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.op.buffer.as_init();
        opcode::SendZc::new(
            Fd(self.op.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .flags(self.op.flags)
        .build()
        .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoBuf, S: AsFd> SendToZc<T, S> {
    /// Create [`SendToZc`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            op: SendTo::new(fd, buffer, addr, flags),
            res: None,
        }
    }
}

impl<T: IoBuf, S: AsFd> IntoInner for SendToZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendToZc<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.header.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.op.header.flags as _)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoVectoredBuf, S> SendVectoredZc<T, S> {
    /// Create [`SendVectoredZc`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            op: SendVectored::new(fd, buffer, flags),
            res: None,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectoredZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectoredZc<T, S> {
    type Control = SendVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.op.create_control(ctrl)
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.op.flags as _)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectoredZc<T, S> {
    /// Create [`SendToVectoredZc`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            op: SendToVectored::new(fd, buffer, addr, flags),
            res: None,
        }
    }
}

impl<T: IoVectoredBuf, S: AsFd> IntoInner for SendToVectoredZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectoredZc<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.header.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.op.header.flags as _)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsgZc<T, C, S> {
    /// Create [`SendMsgZc`].
    pub fn new(fd: S, buffer: T, control: C, addr: Option<SockAddr>, flags: i32) -> Self {
        Self {
            op: SendMsg::new(fd, buffer, control, addr, flags),
            res: None,
        }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsgZc<T, C, S> {
    type Inner = (T, C);

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsgZc<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.op.create_control(ctrl)
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.fd.as_fd().as_raw_fd()), &control.msg)
            .flags(self.op.flags as _)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}
