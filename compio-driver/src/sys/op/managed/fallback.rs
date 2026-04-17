use std::io;

use compio_buf::{IntoInner, IoBuf, IoBufMut, SetLen};
use rustix::net::RecvFlags;
use socket2::SockAddr;

use crate::{
    AsFd, BufferPool, BufferRef,
    op::{RecvMsg, TakeBuffer},
    sys::op::{Read, ReadAt, Recv, RecvFrom},
};

/// Read a file at specified position into managed buffer.
pub struct ReadManagedAt<S> {
    pub(crate) op: ReadAt<BufferRef, S>,
}

impl<S> ReadManagedAt<S> {
    /// Create [`ReadManagedAt`].
    pub fn new(fd: S, offset: u64, pool: &BufferPool, len: usize) -> io::Result<Self> {
        Ok(Self {
            op: ReadAt::new(fd, offset, pool.pop()?.with_capacity(len)),
        })
    }
}

impl<S> TakeBuffer for ReadManagedAt<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        Some(self.op.into_inner())
    }
}

/// Read a file into managed buffer.
pub struct ReadManaged<S> {
    pub(crate) op: Read<BufferRef, S>,
}

impl<S> ReadManaged<S> {
    /// Create [`ReadManaged`].
    pub fn new(fd: S, pool: &BufferPool, len: usize) -> io::Result<Self> {
        Ok(Self {
            op: Read::new(fd, pool.pop()?.with_capacity(len)),
        })
    }
}

impl<S> TakeBuffer for ReadManaged<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        Some(self.op.into_inner())
    }
}

/// Receive data from remote into managed buffer.
///
/// It is only used for socket operations. If you want to read from a pipe,
/// use [`ReadManaged`].
pub struct RecvManaged<S> {
    pub(crate) op: Recv<BufferRef, S>,
}

impl<S> RecvManaged<S> {
    /// Create [`RecvManaged`].
    pub fn new(fd: S, pool: &BufferPool, len: usize, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            op: Recv::new(fd, pool.pop()?.with_capacity(len), flags),
        })
    }
}

impl<S> TakeBuffer for RecvManaged<S> {
    type Buffer = BufferRef;

    fn take_buffer(self) -> Option<BufferRef> {
        Some(self.op.into_inner())
    }
}

/// Receive data and source address into managed buffer.
pub struct RecvFromManaged<S: AsFd> {
    pub(crate) op: RecvFrom<BufferRef, S>,
}

impl<S: AsFd> RecvFromManaged<S> {
    /// Create [`RecvFromManaged`].
    pub fn new(fd: S, pool: &BufferPool, len: usize, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            op: RecvFrom::new(fd, pool.pop()?.with_capacity(len), flags),
        })
    }
}

impl<S: AsFd> TakeBuffer for RecvFromManaged<S> {
    type Buffer = (BufferRef, Option<SockAddr>);

    fn take_buffer(self) -> Option<Self::Buffer> {
        Some(self.op.into_inner())
    }
}

/// Receive data into managed buffer, and ancillary data into control buffer.
pub struct RecvMsgManaged<C: IoBufMut, S: AsFd> {
    pub(crate) op: RecvMsg<[BufferRef; 1], C, S>,
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
            op: RecvMsg::new(fd, [pool.pop()?.with_capacity(len)], control, flags),
        })
    }
}

impl<C: IoBufMut, S: AsFd> TakeBuffer for RecvMsgManaged<C, S> {
    type Buffer = ((BufferRef, C), Option<SockAddr>, usize);

    fn take_buffer(self) -> Option<Self::Buffer> {
        let (([buf], control), addr, len) = self.op.into_inner();
        Some(((buf, control), addr, len))
    }
}

/// Read a file at specified position into multiple managed buffers.
pub type ReadMultiAt<S> = ReadManagedAt<S>;
/// Read a file into multiple managed buffers.
pub type ReadMulti<S> = ReadManaged<S>;
/// Receive data from remote into multiple managed buffers.
pub type RecvMulti<S> = RecvManaged<S>;

/// Result of [`RecvFromMulti`].
pub struct RecvFromMultiResult {
    buffer: BufferRef,
    addr: Option<SockAddr>,
}

impl RecvFromMultiResult {
    #[doc(hidden)]
    pub unsafe fn new(_: BufferRef) -> Self {
        unreachable!("should not be called directly")
    }

    /// Get the payload data.
    pub fn data(&self) -> &[u8] {
        self.buffer.as_init()
    }

    /// Get the source address if applicable.
    pub fn addr(&self) -> Option<SockAddr> {
        self.addr.clone()
    }
}

impl IntoInner for RecvFromMultiResult {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data and source address multi times into multiple managed buffers.
pub struct RecvFromMulti<S: AsFd> {
    pub(crate) op: RecvFromManaged<S>,
    pub(crate) len: usize,
}

impl<S: AsFd> RecvFromMulti<S> {
    /// Create [`RecvFromMulti`].
    pub fn new(fd: S, pool: &BufferPool, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            op: RecvFromManaged::new(fd, pool, 0, flags)?,
            len: 0,
        })
    }
}

impl<S: AsFd> TakeBuffer for RecvFromMulti<S> {
    type Buffer = RecvFromMultiResult;

    fn take_buffer(self) -> Option<Self::Buffer> {
        let (mut buffer, addr) = self.op.take_buffer()?;
        unsafe { buffer.advance_to(self.len) };
        Some(RecvFromMultiResult { buffer, addr })
    }
}

/// Result of [`RecvMsgMulti`].
pub struct RecvMsgMultiResult {
    buffer: BufferRef,
    control: BufferRef,
    addr: Option<SockAddr>,
}

impl RecvMsgMultiResult {
    #[doc(hidden)]
    pub unsafe fn new(_: BufferRef, _: usize) -> Self {
        unreachable!("should not be called directly")
    }

    /// Get the payload data.
    pub fn data(&self) -> &[u8] {
        self.buffer.as_init()
    }

    /// Get the source address if applicable.
    pub fn addr(&self) -> Option<SockAddr> {
        self.addr.clone()
    }

    /// Get the ancillary data.
    pub fn ancillary(&self) -> &[u8] {
        self.control.as_init()
    }
}

impl IntoInner for RecvMsgMultiResult {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data, ancillary data and source address multi times into multiple
/// managed buffers.
pub struct RecvMsgMulti<S: AsFd> {
    pub(crate) op: RecvMsgManaged<BufferRef, S>,
    pub(crate) len: usize,
}

impl<S: AsFd> RecvMsgMulti<S> {
    /// Create [`RecvMsgMulti`].
    pub fn new(fd: S, pool: &BufferPool, control_len: usize, flags: RecvFlags) -> io::Result<Self> {
        Ok(Self {
            op: RecvMsgManaged::new(fd, pool, 0, pool.pop()?.with_capacity(control_len), flags)?,
            len: 0,
        })
    }
}

impl<S: AsFd> TakeBuffer for RecvMsgMulti<S> {
    type Buffer = RecvMsgMultiResult;

    fn take_buffer(self) -> Option<Self::Buffer> {
        let ((mut buffer, mut control), addr, control_len) = self.op.take_buffer()?;
        unsafe { buffer.advance_to(self.len) };
        unsafe { control.advance_to(control_len) };
        Some(RecvMsgMultiResult {
            buffer,
            control,
            addr,
        })
    }
}
