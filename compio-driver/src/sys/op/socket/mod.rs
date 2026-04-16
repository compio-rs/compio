use mod_use::mod_use;

#[cfg(unix)]
mod_use![unix];

#[cfg(io_uring)]
mod_use![iour];

#[cfg(windows)]
mod_use![iocp];

#[cfg(polling)]
mod_use![poll];

#[cfg(stub)]
mod_use![stub];

use crate::sys::prelude::*;

/// Connect to a remote address.
pub struct Connect<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddr,
}

/// Close socket fd.
pub struct CloseSocket {
    pub(crate) fd: ManuallyDrop<OwnedFd>,
}

/// Send data to remote.
///
/// If you want to write to a pipe, use [`Write`].
///
/// [`Write`]: crate::op::Write
pub struct Send<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

pub(crate) struct SendToHeader<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddr,
    pub(crate) flags: i32,
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) header: SendToHeader<S>,
    pub(crate) buffer: T,
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    pub(crate) header: SendToHeader<S>,
    pub(crate) buffer: T,
}

/// Send data to specified address accompanied by ancillary data from vectored
/// buffer.
pub struct SendMsg<T: IoVectoredBuf, C: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) control: C,
    pub(crate) addr: Option<SockAddr>,
    pub(crate) flags: i32,
}

/// Receive data from remote.
///
/// If you want to read from a pipe, use [`Read`].
///
/// [`Read`]: crate::op::Read
pub struct Recv<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) flags: i32,
}

pub(crate) struct RecvFromHeader<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddrStorage,
    pub(crate) flags: i32,
    pub(crate) name_len: socklen_t,
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) header: RecvFromHeader<S>,
    pub(crate) buffer: T,
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) header: RecvFromHeader<S>,
    pub(crate) buffer: T,
}

/// Receive data and source address with ancillary data into vectored
/// buffer.
pub struct RecvMsg<T: IoVectoredBufMut, C: IoBufMut, S> {
    pub(crate) addr: SockAddrStorage,
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) control: C,
    pub(crate) flags: i32,
    pub(crate) name_len: socklen_t,
    pub(crate) control_len: usize,
}

impl<S> Connect<S> {
    /// Create [`Connect`]. `fd` should be bound.
    pub fn new(fd: S, addr: SockAddr) -> Self {
        Self { fd, addr }
    }
}

impl CloseSocket {
    /// Create [`CloseSocket`].
    pub fn new(fd: OwnedFd) -> Self {
        Self {
            fd: ManuallyDrop::new(fd),
        }
    }
}

impl<T: IoBuf, S> Send<T, S> {
    /// Create [`Send`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoBuf, S> IntoInner for Send<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    /// Create [`SendVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<S> SendToHeader<S> {
    pub fn new(fd: S, addr: SockAddr, flags: i32) -> Self {
        Self { fd, addr, flags }
    }
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            header: SendToHeader::new(fd, addr, flags),
            buffer,
        }
    }
}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            header: SendToHeader::new(fd, addr, flags),
            buffer,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsg<T, C, S> {
    /// Create [`SendMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C, addr: Option<SockAddr>, flags: i32) -> Self {
        assert!(
            control.buf_len() == 0 || control.buf_ptr().cast::<CmsgHeader>().is_aligned(),
            "misaligned control message buffer"
        );
        Self {
            fd,
            buffer,
            control,
            addr,
            flags,
        }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsg<T, C, S> {
    type Inner = (T, C);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.control)
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> RecvMsg<T, C, S> {
    /// Create [`RecvMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is
    /// misaligned.
    pub fn new(fd: S, buffer: T, control: C, flags: i32) -> Self {
        assert!(
            control.buf_ptr().cast::<CmsgHeader>().is_aligned(),
            "misaligned control message buffer"
        );
        Self {
            addr: SockAddrStorage::zeroed(),
            fd,
            buffer,
            control,
            flags,
            name_len: 0,
            control_len: 0,
        }
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> IntoInner for RecvMsg<T, C, S> {
    type Inner = ((T, C), Option<SockAddr>, usize);

    fn into_inner(self) -> Self::Inner {
        let addr = (self.name_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.name_len) });
        ((self.buffer, self.control), addr, self.control_len)
    }
}

impl<T: IoBufMut, S> Recv<T, S> {
    /// Create [`Recv`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoBufMut, S> IntoInner for Recv<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    /// Create [`RecvVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self { fd, buffer, flags }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<S> RecvFromHeader<S> {
    pub fn new(fd: S, flags: i32) -> Self {
        let addr = SockAddrStorage::zeroed();
        let name_len = addr.size_of();
        Self {
            fd,
            addr,
            flags,
            name_len,
        }
    }

    pub fn into_addr(self) -> Option<SockAddr> {
        (self.name_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.name_len) })
    }
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            header: RecvFromHeader::new(fd, flags),
            buffer,
        }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = self.header.into_addr();
        (self.buffer, addr)
    }
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            header: RecvFromHeader::new(fd, flags),
            buffer,
        }
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.header.into_addr())
    }
}
