use std::io::{IoSlice, IoSliceMut};

use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

#[cfg(doc)]
use crate::op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IntoInner, OneOrVec},
    driver::RawFd,
};

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: RawFd,
    pub(crate) buffer: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
}

impl Accept {
    /// Create [`Accept`].
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            buffer: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
        }
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> SockAddr {
        unsafe { SockAddr::new(self.buffer, self.addr_len) }
    }
}

/// Receive data from remote.
pub struct RecvImpl<'slice, T: AsIoSlicesMut + Unpin + 'slice> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: OneOrVec<IoSliceMut<'slice>>,
}

impl<'slice, T: AsIoSlicesMut + Unpin> RecvImpl<'slice, T> {
    /// Create [`Recv`] or [`RecvVectored`].
    pub fn new(fd: RawFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            slices: OneOrVec::One(IoSliceMut::new(&mut [])),
        }
    }
}

impl<'slice, T: AsIoSlicesMut + Unpin> IntoInner for RecvImpl<'slice, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote.
pub struct SendImpl<'slice, T: AsIoSlices + Unpin + 'slice> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: OneOrVec<IoSlice<'slice>>,
}

impl<'slice, T: AsIoSlices + Unpin> SendImpl<'slice, T> {
    /// Create [`Send`] or [`SendVectored`].
    pub fn new(fd: RawFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            slices: OneOrVec::One(IoSlice::new(&[])),
        }
    }
}

impl<'slice, T: AsIoSlices + Unpin + 'slice> IntoInner for SendImpl<'slice, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data and source address.
pub struct RecvFromImpl<'slice, T: AsIoSlicesMut + Unpin + 'slice> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) slices: OneOrVec<IoSliceMut<'slice>>,
    pub(crate) msg: libc::msghdr,
}

impl<'slice, T: AsIoSlicesMut + Unpin + 'slice> RecvFromImpl<'slice, T> {
    /// Create [`RecvFrom`] or [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            addr: unsafe { std::mem::zeroed() },
            slices: OneOrVec::One(IoSliceMut::new(&mut [])),
            msg: unsafe { std::mem::zeroed() },
        }
    }

    pub(crate) fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: std::mem::size_of_val(&self.addr) as _,
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
    }
}

impl<'slice, T: AsIoSlicesMut + Unpin + 'slice> IntoInner for RecvFromImpl<'slice, T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

/// Send data to specified address.
pub struct SendToImpl<'slice, T: AsIoSlices + Unpin + 'slice> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) slices: OneOrVec<IoSlice<'slice>>,
    pub(crate) msg: libc::msghdr,
}

impl<'slice, T: AsIoSlices + Unpin + 'slice> SendToImpl<'slice, T> {
    /// Create [`SendTo`] or [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T::Inner, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            addr,
            slices: OneOrVec::One(IoSlice::new(&[])),
            msg: unsafe { std::mem::zeroed() },
        }
    }

    pub(crate) fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.as_io_slices() };
        self.msg = libc::msghdr {
            msg_name: self.addr.as_ptr() as _,
            msg_namelen: self.addr.len(),
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
    }
}

impl<'slice, T: AsIoSlices + Unpin + 'slice> IntoInner for SendToImpl<'slice, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
