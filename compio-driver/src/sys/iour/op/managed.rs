use std::{
    collections::VecDeque,
    io,
    mem::ManuallyDrop,
    os::fd::{AsFd, AsRawFd},
    ptr::{self, drop_in_place},
};

use compio_buf::BufResult;
use io_uring::{opcode, squeue::Flags, types::Fd};
use socket2::{SockAddr, SockAddrStorage, socklen_t};

use super::{Extra, OpCode};
use crate::{BufferPool, BufferRef, OpEntry, op::TakeBuffer};

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

    unsafe fn init(&mut self) -> Self::Control {}

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

    unsafe fn init(&mut self) -> Self::Control {}

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
    flags: i32,
    buffer_group: u16,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
}

impl<S> RecvManaged<S> {
    /// Create [`RecvManaged`].
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
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

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        opcode::Recv::new(Fd(fd), ptr::null_mut(), self.len)
            .flags(self.flags)
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
    flags: i32,
    addr: SockAddrStorage,
    addr_len: socklen_t,
    iovec: libc::iovec,
    msg: libc::msghdr,
    buffer_group: u16,
    buffer_pool: BufferPool,
    buffer: Option<BufferRef>,
}

impl<S> RecvFromManaged<S> {
    /// Create [`RecvFromManaged`].
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
        let len: u32 = len
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "required length too long"))?;
        let addr = SockAddrStorage::zeroed();
        Ok(Self {
            fd,
            buffer_group: buffer_pool.buffer_group()?,
            flags,
            addr_len: addr.size_of() as _,
            addr,
            iovec: libc::iovec {
                iov_base: ptr::null_mut(),
                iov_len: len as _,
            },
            msg: unsafe { std::mem::zeroed() },
            buffer_pool: buffer_pool.clone(),
            buffer: None,
        })
    }
}

impl<S> TakeBuffer for RecvFromManaged<S> {
    type Buffer = (BufferRef, Option<SockAddr>);

    fn take_buffer(self) -> Option<Self::Buffer> {
        let buf = self.buffer?;
        let addr = (self.msg.msg_namelen > 0)
            .then(|| unsafe { SockAddr::new(self.addr, self.msg.msg_namelen) });
        Some((buf, addr))
    }
}

unsafe impl<S: AsFd> OpCode for RecvFromManaged<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        self.msg.msg_name = &raw mut self.addr as _;
        self.msg.msg_namelen = self.addr_len;
        self.msg.msg_iov = &raw mut self.iovec;
        self.msg.msg_iovlen = 1;
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut self.msg)
            .flags(self.flags as _)
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

    unsafe fn init(&mut self) -> Self::Control {}

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

    unsafe fn init(&mut self) -> Self::Control {}

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
    pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
        Ok(Self {
            inner: RecvManaged::new(fd, buffer_pool, len, flags)?,
            multishots: VecDeque::new(),
        })
    }
}

unsafe impl<S: AsFd> OpCode for RecvMulti<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        let fd = self.inner.fd.as_fd().as_raw_fd();
        opcode::RecvMulti::new(Fd(fd), self.inner.buffer_group)
            .flags(self.inner.flags)
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

impl<S> TakeBuffer for RecvMulti<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        self.inner.take_buffer()
    }
}
