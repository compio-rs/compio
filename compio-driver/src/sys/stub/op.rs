#![allow(dead_code)]

use std::ffi::CString;

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use socket2::{SockAddr, SockAddrStorage, Socket as Socket2, socklen_t};

pub use self::{
    Send as SendZc, SendMsg as SendMsgZc, SendTo as SendToZc, SendToVectored as SendToVectoredZc,
    SendVectored as SendVectoredZc,
};
use super::{OpCode, stub_unimpl};
pub use crate::sys::unix_op::*;
use crate::{AsFd, op::*};

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S1, S2, D, F> OpCode for AsyncifyFd2<S1, S2, F, D>
where
    S1: std::marker::Sync,
    S2: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

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

impl<S: AsFd> OpCode for FileStat<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

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

impl<S: AsFd> OpCode for PathStat<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Sync<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl OpCode for CreateSocket {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl OpCode for CloseSocket {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

/// Accept multiple connections.
pub struct AcceptMulti<S> {
    fd: S,
}

impl<S> AcceptMulti<S> {
    /// Create [`AcceptMulti`].
    pub fn new(fd: S) -> Self {
        Self { fd }
    }
}

impl<S> IntoInner for AcceptMulti<S> {
    type Inner = Socket2;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

impl<S: AsFd> OpCode for AcceptMulti<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

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

impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

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

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

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

impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

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

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl OpCode for Pipe {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for crate::op::managed::ReadManagedAt<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for crate::op::managed::ReadManaged<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for crate::op::managed::RecvManaged<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}

impl<S: AsFd> OpCode for crate::op::managed::RecvFromManaged<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}
}
