use std::io;

use compio_buf::IntoInner;
use socket2::SockAddr;

use super::{Read, ReadAt, Recv, RecvFrom};
use crate::{AsFd, BufferPool, BufferRef, op::TakeBuffer};

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
    pub fn new(fd: S, pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
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
    pub fn new(fd: S, pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
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

/// Read a file at specified position into multiple managed buffers.
pub type ReadMultiAt<S> = ReadManagedAt<S>;
/// Read a file into multiple managed buffers.
pub type ReadMulti<S> = ReadManaged<S>;
/// Receive data from remote into multiple managed buffers.
pub type RecvMulti<S> = RecvManaged<S>;
