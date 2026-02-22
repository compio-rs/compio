#![allow(dead_code)]

use std::ffi::CString;

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use socket2::{SockAddr, SockAddrStorage, socklen_t};

use super::{OpCode, stub_unimpl};
pub use crate::sys::unix_op::*;
use crate::{AsFd, op::*};

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
}

impl<
    S,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd<S, F, D>
{
}

impl<
    S1,
    S2,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd2<S1, S2, F, D>
{
}

impl<S: AsFd> OpCode for OpenFile<S> {}

impl OpCode for CloseFile {}

impl<S: AsFd> OpCode for TruncateFile<S> {}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
}

impl<S> FileStat<S> {
    /// Create [`FileStat`].
    pub fn new(fd: S) -> Self {
        Self { fd }
    }
}

impl<S: AsFd> OpCode for FileStat<S> {}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

/// Get metadata from path.
pub struct PathStat<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) follow_symlink: bool,
}

impl<S: AsFd> PathStat<S> {
    /// Create [`PathStat`].
    pub fn new(dirfd: S, path: CString, follow_symlink: bool) -> Self {
        Self {
            dirfd,
            path,
            follow_symlink,
        }
    }
}

impl<S: AsFd> OpCode for PathStat<S> {}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {}

impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {}

impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {}

impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {}

impl<S: AsFd> OpCode for Sync<S> {}

impl<S: AsFd> OpCode for Unlink<S> {}

impl<S: AsFd> OpCode for CreateDir<S> {}

impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {}

impl<S: AsFd> OpCode for Symlink<S> {}

impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {}

impl OpCode for CreateSocket {}

impl<S: AsFd> OpCode for ShutdownSocket<S> {}

impl OpCode for CloseSocket {}

impl<S: AsFd> OpCode for Accept<S> {}

impl<S: AsFd> OpCode for Connect<S> {}

impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {}

impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    fd: S,
    buffer: T,
    flags: i32,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {}

impl<T: IoBufMut, S: AsFd> IntoInner for RecvFrom<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    fd: S,
    buffer: T,
    flags: i32,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {}

impl<T: IoVectoredBufMut, S: AsFd> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    fd: S,
    buffer: T,
    addr: SockAddr,
    flags: i32,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
        }
    }
}

impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    fd: S,
    buffer: T,
    addr: SockAddr,
    flags: i32,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
        }
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {}

impl<S: AsFd> OpCode for PollOnce<S> {}

impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {}

impl<S: AsFd> OpCode for crate::op::managed::ReadManagedAt<S> {}

impl<S: AsFd> OpCode for crate::op::managed::ReadManaged<S> {}

impl<S: AsFd> OpCode for crate::op::managed::RecvManaged<S> {}
