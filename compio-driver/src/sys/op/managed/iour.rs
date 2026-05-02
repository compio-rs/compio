use std::{
    collections::VecDeque,
    io,
    mem::{ManuallyDrop, size_of},
    os::fd::{AsFd, AsRawFd},
    ptr::{self, drop_in_place},
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetLen};
use io_uring::{opcode, squeue::Flags, types::Fd};
use rustix::net::RecvFlags;
use socket2::{SockAddr, SockAddrStorage, socklen_t};

use crate::{
    BufferPool, BufferRef, Extra, IourOpCode as OpCode, OpEntry, op::TakeBuffer,
    sys::pal::is_kernel_newer_than,
};

/// Read a file at specified position into specified buffer.
pub struct ReadManagedAt<S> {
    pub(crate) fd: S,
    pub(crate) offset: u64,
    buffer_group: u16,
    len: u32,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
}

impl<S> ReadManagedAt<S> {
    /// Create [`ReadManagedAt`].
    pub fn new(fd: S, offset: u64, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
        Ok(Self {
            fd,
            offset,
            buffer_group: buffer_pool.buffer_group()?,
            len: len.try_into().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
            })?,
            buffer_pool: buffer_pool.clone(),
            buffer: None,
        })
    }
}

unsafe impl<S: AsFd> OpCode for ReadManagedAt<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = Fd(self.fd.as_fd().as_raw_fd());
        let offset = self.offset;
        opcode::Read::new(fd, ptr::null_mut(), self.len)
            .offset(offset)
            .buf_group(self.buffer_group)
            .build()
            .flags(Flags::BUFFER_SELECT)
            .into()
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, _: &io::Result<usize>, extra: &Extra) {
        let Ok(buffer_id) = extra.buffer_id() else {
            return;
        };
        let buffer = self
            .buffer_pool
            .take(buffer_id)
            .expect("Driver should be alive")
            .expect("Buffer should not be in use");
        self.buffer.replace(buffer);
    }
}

impl<S> TakeBuffer for ReadManagedAt<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.buffer
    }
}

/// Read a file.
pub struct ReadManaged<S> {
    fd: S,
    len: u32,
    buffer_group: u16,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
}

impl<S> ReadManaged<S> {
    /// Create [`ReadManaged`].
    pub fn new(fd: S, pool: &BufferPool, len: usize) -> io::Result<Self> {
        Ok(Self {
            fd,
            buffer_group: pool.buffer_group()?,
            len: len.try_into().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
            })?,
            buffer_pool: pool.clone(),
            buffer: None,
        })
    }
}

unsafe impl<S: AsFd> OpCode for ReadManaged<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        opcode::Read::new(Fd(fd), ptr::null_mut(), self.len)
            .buf_group(self.buffer_group)
            .offset(u64::MAX)
            .build()
            .flags(Flags::BUFFER_SELECT)
            .into()
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, _: &io::Result<usize>, extra: &Extra) {
        let Ok(buffer_id) = extra.buffer_id() else {
            return;
        };
        let buffer = self
            .buffer_pool
            .take(buffer_id)
            .expect("Driver should be alive")
            .expect("Buffer should not be in use");
        self.buffer.replace(buffer);
    }
}

impl<S> TakeBuffer for ReadManaged<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.buffer
    }
}

/// Receive data from remote.
pub struct RecvManaged<S> {
    fd: S,
    len: u32,
    flags: RecvFlags,
    buffer_group: u16,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
}

impl<S> RecvManaged<S> {
    /// Create [`RecvManaged`].
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            fd,
            buffer_group: buffer_pool.buffer_group()?,
            len: len.try_into().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
            })?,
            flags,
            buffer_pool: buffer_pool.clone(),
            buffer: None,
        })
    }
}

unsafe impl<S: AsFd> OpCode for RecvManaged<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        opcode::Recv::new(Fd(fd), ptr::null_mut(), self.len)
            .flags(self.flags.bits() as _)
            .buf_group(self.buffer_group)
            .build()
            .flags(Flags::BUFFER_SELECT)
            .into()
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, _: &io::Result<usize>, extra: &Extra) {
        let Ok(buffer_id) = extra.buffer_id() else {
            return;
        };
        let buffer = self
            .buffer_pool
            .take(buffer_id)
            .expect("Driver should be alive")
            .expect("Buffer should not be in use");
        self.buffer.replace(buffer);
    }
}

impl<S> TakeBuffer for RecvManaged<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.buffer
    }
}

/// Receive data and source address into managed buffer.
pub struct RecvFromManaged<S> {
    fd: S,
    flags: RecvFlags,
    addr: SockAddrStorage,
    name_len: socklen_t,
    buffer_len: usize,
    buffer_group: u16,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
}

#[doc(hidden)]
pub struct RecvFromManagedControl {
    msg: libc::msghdr,
    #[allow(dead_code)]
    iovec: libc::iovec,
}

impl Default for RecvFromManagedControl {
    fn default() -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            iovec: unsafe { std::mem::zeroed() },
        }
    }
}

impl<S> RecvFromManaged<S> {
    /// Create [`RecvFromManaged`].
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: RecvFlags) -> io::Result<Self> {
        let addr = SockAddrStorage::zeroed();
        Ok(Self {
            fd,
            buffer_group: buffer_pool.buffer_group()?,
            flags,
            name_len: 0,
            buffer_len: len,
            addr,
            buffer_pool: buffer_pool.clone(),
            buffer: None,
        })
    }
}

impl<S> TakeBuffer for RecvFromManaged<S> {
    type Buffer = (BufferRef, Option<SockAddr>);

    fn take_buffer(self) -> Option<Self::Buffer> {
        let buf = self.buffer?;
        let addr = (self.name_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.name_len) });
        Some((buf, addr))
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromManaged<S> {
    type Control = RecvFromManagedControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.iovec.iov_len = self.buffer_len;
        ctrl.msg.msg_name = &raw mut self.addr as _;
        ctrl.msg.msg_namelen = self.addr.size_of() as _;
        ctrl.msg.msg_iov = &raw mut ctrl.iovec;
        ctrl.msg.msg_iovlen = 1;
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut control.msg)
            .flags(self.flags.bits() as _)
            .buf_group(self.buffer_group)
            .build()
            .flags(Flags::BUFFER_SELECT)
            .into()
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        extra: &Extra,
    ) {
        self.name_len = control.msg.msg_namelen;
        let Ok(buffer_id) = extra.buffer_id() else {
            return;
        };
        let buffer = self
            .buffer_pool
            .take(buffer_id)
            .expect("Driver should be alive")
            .expect("Buffer should not be in use");
        self.buffer.replace(buffer);
    }
}

/// Receive data into managed buffer, and ancillary data into control buffer.
pub struct RecvMsgManaged<C: IoBufMut, S: AsFd> {
    op: RecvFromManaged<S>,
    control: C,
    control_len: usize,
}

impl<C: IoBufMut, S: AsFd> RecvMsgManaged<C, S> {
    /// Create [`RecvMsgManaged`].
    pub fn new(
        fd: S,
        pool: &BufferPool,
        len: usize,
        control: C,
        flags: RecvFlags,
    ) -> io::Result<Self> {
        Ok(Self {
            op: RecvFromManaged::new(fd, pool, len, flags)?,
            control,
            control_len: 0,
        })
    }
}

unsafe impl<C: IoBufMut, S: AsFd> OpCode for RecvMsgManaged<C, S> {
    type Control = RecvFromManagedControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) };
        let slice = self.control.as_uninit();
        ctrl.msg.msg_control = slice.as_mut_ptr() as _;
        ctrl.msg.msg_controllen = slice.len() as _;
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &Extra,
    ) {
        unsafe { self.op.set_result(control, result, extra) };
        self.control_len = control.msg.msg_controllen as _;
    }
}

impl<C: IoBufMut, S: AsFd> TakeBuffer for RecvMsgManaged<C, S> {
    type Buffer = ((BufferRef, C), Option<SockAddr>, usize);

    fn take_buffer(self) -> Option<Self::Buffer> {
        let (buffer, addr) = self.op.take_buffer()?;
        Some(((buffer, self.control), addr, self.control_len))
    }
}

struct BufferGuard {
    pool: BufferPool,
    buffer_id: u16,
}

impl BufferGuard {
    pub fn leak(self) {
        let mut this = ManuallyDrop::new(self);
        // SAFETY: we're taking ownership of self, so this function will be executed
        // at most once
        unsafe { drop_in_place(&raw mut this.pool) }
    }
}

impl Drop for BufferGuard {
    fn drop(&mut self) {
        _ = self.pool.reset(self.buffer_id);
    }
}

struct MultishotResult {
    result: io::Result<usize>,
    extra: Extra,
    guard: Option<BufferGuard>,
}

impl MultishotResult {
    pub fn new(result: io::Result<usize>, extra: Extra, pool: &BufferPool) -> Self {
        let guard = extra.buffer_id().ok().map(|buffer_id| BufferGuard {
            pool: pool.clone(),
            buffer_id,
        });
        Self {
            result,
            extra,
            guard,
        }
    }

    pub fn into_result(mut self) -> BufResult<usize, Extra> {
        if let Some(guard) = self.guard.take() {
            guard.leak();
        };
        BufResult(self.result, self.extra)
    }
}

/// Read a file at specified position into multiple managed buffers.
pub struct ReadMultiAt<S> {
    inner: ReadManagedAt<S>,
    multishots: VecDeque<MultishotResult>,
}

impl<S> ReadMultiAt<S> {
    /// Create [`ReadMultiAt`].
    pub fn new(fd: S, offset: u64, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
        Ok(Self {
            inner: ReadManagedAt::new(fd, offset, buffer_pool, len)?,
            multishots: VecDeque::new(),
        })
    }
}

unsafe impl<S: AsFd> OpCode for ReadMultiAt<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.inner.fd.as_fd().as_raw_fd();
        opcode::ReadMulti::new(Fd(fd), self.inner.len, self.inner.buffer_group)
            .offset(self.inner.offset)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.inner.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.inner.set_result(control, res, extra) }
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.multishots
            .push_back(MultishotResult::new(res, extra, &self.inner.buffer_pool));
    }

    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.multishots
            .pop_front()
            .map(MultishotResult::into_result)
    }
}

impl<S> TakeBuffer for ReadMultiAt<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.inner.take_buffer()
    }
}

/// Read a file into multiple managed buffers.
pub struct ReadMulti<S> {
    inner: ReadManaged<S>,
    multishots: VecDeque<MultishotResult>,
}

impl<S> ReadMulti<S> {
    /// Create [`ReadMulti`].
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
        Ok(Self {
            inner: ReadManaged::new(fd, buffer_pool, len)?,
            multishots: VecDeque::new(),
        })
    }
}

unsafe impl<S: AsFd> OpCode for ReadMulti<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.inner.fd.as_fd().as_raw_fd();
        opcode::ReadMulti::new(Fd(fd), self.inner.len, self.inner.buffer_group)
            .offset(u64::MAX)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.inner.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.inner.set_result(control, res, extra) }
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.multishots
            .push_back(MultishotResult::new(res, extra, &self.inner.buffer_pool));
    }

    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.multishots
            .pop_front()
            .map(MultishotResult::into_result)
    }
}

impl<S> TakeBuffer for ReadMulti<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.inner.take_buffer()
    }
}

/// Receive data from remote into multiple managed buffers.
pub struct RecvMulti<S> {
    inner: RecvManaged<S>,
    multishots: VecDeque<MultishotResult>,
}

impl<S> RecvMulti<S> {
    /// Create [`RecvMulti`].
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            inner: RecvManaged::new(fd, buffer_pool, len, flags)?,
            multishots: VecDeque::new(),
        })
    }
}

unsafe impl<S: AsFd> OpCode for RecvMulti<S> {
    type Control = ();

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        if is_kernel_newer_than((6, 0, 0)) {
            let fd = self.inner.fd.as_fd().as_raw_fd();
            opcode::RecvMulti::new(Fd(fd), self.inner.buffer_group)
                .flags(self.inner.flags.bits() as _)
                .len(self.inner.len)
                .build()
                .into()
        } else {
            self.create_entry_fallback(control)
        }
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.inner.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.inner.set_result(control, res, extra) }
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.multishots
            .push_back(MultishotResult::new(res, extra, &self.inner.buffer_pool));
    }

    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.multishots
            .pop_front()
            .map(MultishotResult::into_result)
    }
}

impl<S> TakeBuffer for RecvMulti<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.inner.take_buffer()
    }
}

#[derive(Debug)]
#[repr(C)]
#[allow(non_camel_case_types)]
struct io_uring_recvmsg_out {
    namelen: u32,
    controllen: u32,
    payloadlen: u32,
    flags: u32,
}

struct RecvMsgMultiResultImpl {
    buffer: BufferRef,
    clen: usize,
}

const NLEN: usize = size_of::<SockAddrStorage>();

impl RecvMsgMultiResultImpl {
    unsafe fn new(buffer: BufferRef, clen: usize) -> Self {
        assert!(buffer.len() >= size_of::<io_uring_recvmsg_out>());
        let header = unsafe {
            buffer
                .as_init()
                .as_ptr()
                .cast::<io_uring_recvmsg_out>()
                .read_unaligned()
        };
        let total_len =
            size_of::<io_uring_recvmsg_out>() + NLEN + clen + header.payloadlen as usize;
        assert!(buffer.len() >= total_len);
        Self { buffer, clen }
    }

    fn header(&self) -> io_uring_recvmsg_out {
        // SAFETY: we provide enough capacity for the header
        unsafe {
            self.buffer
                .as_ptr()
                .cast::<io_uring_recvmsg_out>()
                .read_unaligned()
        }
    }

    fn data(&self) -> &[u8] {
        let offset = size_of::<io_uring_recvmsg_out>() + NLEN + self.clen;
        &self.buffer.as_init()[offset..]
    }

    fn addr(&self) -> Option<SockAddr> {
        let header = self.header();
        if header.namelen == 0 {
            None
        } else {
            let offset = size_of::<io_uring_recvmsg_out>();
            let mut addr = SockAddrStorage::zeroed();
            unsafe {
                ptr::copy_nonoverlapping(
                    self.buffer.as_ptr().add(offset),
                    &raw mut addr as *mut u8,
                    header.namelen as usize,
                );
            }
            Some(unsafe { SockAddr::new(addr, header.namelen as _) })
        }
    }

    fn ancillary(&self) -> &[u8] {
        let header = self.header();
        let offset = size_of::<io_uring_recvmsg_out>() + NLEN;
        &self.buffer.as_init()[offset..offset + header.controllen as usize]
    }
}

impl IntoInner for RecvMsgMultiResultImpl {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

struct RecvMsgMultiImpl<S: AsFd> {
    fd: S,
    flags: RecvFlags,
    control_len: usize,
    buffer_group: u16,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
    multishots: VecDeque<MultishotResult>,
    len: usize,
}

impl<S: AsFd> RecvMsgMultiImpl<S> {
    pub fn new(
        fd: S,
        buffer_pool: &BufferPool,
        control_len: usize,
        flags: RecvFlags,
    ) -> io::Result<Self> {
        Ok(Self {
            fd,
            buffer_group: buffer_pool.buffer_group()?,
            flags,
            control_len,
            buffer_pool: buffer_pool.clone(),
            buffer: None,
            multishots: VecDeque::new(),
            len: 0,
        })
    }
}

impl<S: AsFd> TakeBuffer for RecvMsgMultiImpl<S> {
    type Buffer = RecvMsgMultiResultImpl;

    fn take_buffer(self) -> Option<Self::Buffer> {
        let mut buffer = self.buffer?;
        unsafe { buffer.advance_to(self.len) };
        Some(unsafe { RecvMsgMultiResultImpl::new(buffer, self.control_len) })
    }
}

unsafe impl<S: AsFd> OpCode for RecvMsgMultiImpl<S> {
    type Control = RecvFromManagedControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.msg.msg_namelen = NLEN as _;
        ctrl.msg.msg_controllen = self.control_len as _;
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::RecvMsgMulti::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            &raw mut control.msg,
            self.buffer_group,
        )
        .flags(self.flags.bits() as _)
        .build()
        .into()
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.multishots
            .push_back(MultishotResult::new(res, extra, &self.buffer_pool));
    }

    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.multishots
            .pop_front()
            .map(MultishotResult::into_result)
    }

    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &Extra,
    ) {
        let Ok(buffer_id) = extra.buffer_id() else {
            return;
        };
        let buffer = self
            .buffer_pool
            .take(buffer_id)
            .expect("Driver should be alive")
            .expect("Buffer should not be in use");
        self.buffer.replace(buffer);
        if let Ok(result) = result {
            self.len = *result;
        }
    }
}

struct RecvMsgMultiResultFallback {
    buffer: BufferRef,
    control: BufferRef,
    addr: Option<SockAddr>,
}

impl RecvMsgMultiResultFallback {
    fn data(&self) -> &[u8] {
        self.buffer.as_init()
    }

    fn addr(&self) -> Option<SockAddr> {
        self.addr.clone()
    }

    fn ancillary(&self) -> &[u8] {
        self.control.as_init()
    }
}

impl IntoInner for RecvMsgMultiResultFallback {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

struct RecvMsgMultiFallback<S: AsFd> {
    op: RecvMsgManaged<BufferRef, S>,
    len: usize,
}

impl<S: AsFd> RecvMsgMultiFallback<S> {
    pub fn new(fd: S, pool: &BufferPool, control_len: usize, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            op: RecvMsgManaged::new(fd, pool, 0, pool.pop()?.with_capacity(control_len), flags)?,
            len: 0,
        })
    }
}

impl<S: AsFd> TakeBuffer for RecvMsgMultiFallback<S> {
    type Buffer = RecvMsgMultiResultFallback;

    fn take_buffer(self) -> Option<Self::Buffer> {
        let ((mut buffer, mut control), addr, control_len) = self.op.take_buffer()?;
        unsafe { buffer.advance_to(self.len) };
        unsafe { control.advance_to(control_len) };
        Some(RecvMsgMultiResultFallback {
            buffer,
            control,
            addr,
        })
    }
}

unsafe impl<S: AsFd> OpCode for RecvMsgMultiFallback<S> {
    type Control = RecvFromManagedControl;

    unsafe fn init(&mut self, control: &mut Self::Control) {
        unsafe { self.op.init(control) }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        if let Ok(len) = result {
            self.len = *len;
        }
        unsafe { self.op.set_result(control, result, extra) };
    }
}

enum RecvMsgMultiResultInner {
    Impl(RecvMsgMultiResultImpl),
    Fallback(RecvMsgMultiResultFallback),
}

impl RecvMsgMultiResultInner {
    unsafe fn new(buffer: BufferRef, clen: usize) -> Self {
        Self::Impl(unsafe { RecvMsgMultiResultImpl::new(buffer, clen) })
    }

    fn data(&self) -> &[u8] {
        match self {
            Self::Impl(inner) => inner.data(),
            Self::Fallback(inner) => inner.data(),
        }
    }

    fn addr(&self) -> Option<SockAddr> {
        match self {
            Self::Impl(inner) => inner.addr(),
            Self::Fallback(inner) => inner.addr(),
        }
    }

    fn ancillary(&self) -> &[u8] {
        match self {
            Self::Impl(inner) => inner.ancillary(),
            Self::Fallback(inner) => inner.ancillary(),
        }
    }
}

impl IntoInner for RecvMsgMultiResultInner {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        match self {
            Self::Impl(inner) => inner.into_inner(),
            Self::Fallback(inner) => inner.into_inner(),
        }
    }
}

/// Result of [`RecvMsgMulti`].
pub struct RecvMsgMultiResult {
    inner: RecvMsgMultiResultInner,
}

impl RecvMsgMultiResult {
    /// Create [`RecvMsgMultiResult`] from a buffer received from
    /// [`RecvMsgMulti`].
    ///
    /// # Safety
    ///
    /// The buffer must be received from [`RecvMsgMulti`] or have the same
    /// format as the buffer received from [`RecvMsgMulti`].
    pub unsafe fn new(buffer: BufferRef, clen: usize) -> Self {
        Self {
            inner: unsafe { RecvMsgMultiResultInner::new(buffer, clen) },
        }
    }

    /// Get the payload data.
    pub fn data(&self) -> &[u8] {
        self.inner.data()
    }

    /// Get the source address if applicable.
    pub fn addr(&self) -> Option<SockAddr> {
        self.inner.addr()
    }

    /// Get the ancillary data.
    pub fn ancillary(&self) -> &[u8] {
        self.inner.ancillary()
    }
}

impl IntoInner for RecvMsgMultiResult {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        self.inner.into_inner()
    }
}

enum RecvMsgMultiInner<S: AsFd> {
    Impl(RecvMsgMultiImpl<S>),
    Fallback(RecvMsgMultiFallback<S>),
}

/// Receive data, ancillary data and source address multi times into
/// multiple managed buffers.
pub struct RecvMsgMulti<S: AsFd> {
    inner: RecvMsgMultiInner<S>,
}

impl<S: AsFd> RecvMsgMulti<S> {
    /// Create [`RecvMsgMulti`].
    pub fn new(fd: S, pool: &BufferPool, control_len: usize, flags: RecvFlags) -> io::Result<Self> {
        let inner = if is_kernel_newer_than((6, 0, 0)) {
            RecvMsgMultiInner::Impl(RecvMsgMultiImpl::new(fd, pool, control_len, flags)?)
        } else {
            RecvMsgMultiInner::Fallback(RecvMsgMultiFallback::new(fd, pool, control_len, flags)?)
        };
        Ok(Self { inner })
    }
}

impl<S: AsFd> TakeBuffer for RecvMsgMulti<S> {
    type Buffer = RecvMsgMultiResult;

    fn take_buffer(self) -> Option<Self::Buffer> {
        let res = match self.inner {
            RecvMsgMultiInner::Impl(inner) => RecvMsgMultiResultInner::Impl(inner.take_buffer()?),
            RecvMsgMultiInner::Fallback(inner) => {
                RecvMsgMultiResultInner::Fallback(inner.take_buffer()?)
            }
        };
        Some(RecvMsgMultiResult { inner: res })
    }
}

unsafe impl<S: AsFd> OpCode for RecvMsgMulti<S> {
    type Control = RecvFromManagedControl;

    unsafe fn init(&mut self, control: &mut Self::Control) {
        match &mut self.inner {
            RecvMsgMultiInner::Impl(inner) => unsafe { inner.init(control) },
            RecvMsgMultiInner::Fallback(inner) => unsafe { inner.init(control) },
        }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        match &mut self.inner {
            RecvMsgMultiInner::Impl(inner) => inner.create_entry(control),
            RecvMsgMultiInner::Fallback(inner) => inner.create_entry(control),
        }
    }

    unsafe fn push_multishot(
        &mut self,
        control: &mut Self::Control,
        result: io::Result<usize>,
        extra: crate::Extra,
    ) {
        unsafe {
            match &mut self.inner {
                RecvMsgMultiInner::Impl(inner) => inner.push_multishot(control, result, extra),
                RecvMsgMultiInner::Fallback(inner) => inner.push_multishot(control, result, extra),
            }
        }
    }

    fn pop_multishot(
        &mut self,
        control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        match &mut self.inner {
            RecvMsgMultiInner::Impl(inner) => inner.pop_multishot(control),
            RecvMsgMultiInner::Fallback(inner) => inner.pop_multishot(control),
        }
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &Extra,
    ) {
        unsafe {
            match &mut self.inner {
                RecvMsgMultiInner::Impl(inner) => inner.set_result(control, result, extra),
                RecvMsgMultiInner::Fallback(inner) => inner.set_result(control, result, extra),
            }
        }
    }
}

/// Result of [`RecvFromMulti`].
pub struct RecvFromMultiResult {
    inner: RecvMsgMultiResultInner,
}

impl RecvFromMultiResult {
    /// Create [`RecvFromMultiResult`] from a buffer received from
    /// [`RecvFromMulti`].
    ///
    /// # Safety
    ///
    /// The buffer must be received from [`RecvFromMulti`] or have the same
    /// format as the buffer received from [`RecvFromMulti`].
    pub unsafe fn new(buffer: BufferRef) -> Self {
        Self {
            inner: unsafe { RecvMsgMultiResultInner::new(buffer, 0) },
        }
    }

    /// Get the payload data.
    pub fn data(&self) -> &[u8] {
        self.inner.data()
    }

    /// Get the source address if applicable.
    pub fn addr(&self) -> Option<SockAddr> {
        self.inner.addr()
    }
}

impl IntoInner for RecvFromMultiResult {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        self.inner.into_inner()
    }
}

/// Receive data and source address multi times into multiple managed buffers.
pub struct RecvFromMulti<S: AsFd> {
    op: RecvMsgMulti<S>,
}

impl<S: AsFd> RecvFromMulti<S> {
    /// Create [`RecvFromMulti`].
    pub fn new(fd: S, pool: &BufferPool, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            op: RecvMsgMulti::new(fd, pool, 0, flags)?,
        })
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromMulti<S> {
    type Control = RecvFromManagedControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        control: &mut Self::Control,
        result: io::Result<usize>,
        extra: crate::Extra,
    ) {
        unsafe {
            self.op.push_multishot(control, result, extra);
        }
    }

    fn pop_multishot(
        &mut self,
        control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.op.pop_multishot(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &Extra,
    ) {
        unsafe { self.op.set_result(control, result, extra) };
    }
}

impl<S: AsFd> TakeBuffer for RecvFromMulti<S> {
    type Buffer = RecvFromMultiResult;

    fn take_buffer(self) -> Option<Self::Buffer> {
        let res = self.op.take_buffer()?;
        Some(RecvFromMultiResult { inner: res.inner })
    }
}
