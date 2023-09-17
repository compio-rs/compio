use std::marker::PhantomData;

use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

#[cfg(doc)]
use crate::op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IntoInner},
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
pub struct RecvImpl<'arena, T: AsIoSlicesMut<'arena>> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena, T: AsIoSlicesMut<'arena>> RecvImpl<'arena, T> {
    /// Create [`Recv`] or [`RecvVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _lifetime: PhantomData,
        }
    }
}

impl<'arena, T: AsIoSlicesMut<'arena>> IntoInner for RecvImpl<'arena, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote.
pub struct SendImpl<'arena, T: AsIoSlices<'arena>> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena, T: AsIoSlices<'arena>> SendImpl<'arena, T> {
    /// Create [`Send`] or [`SendVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _lifetime: PhantomData,
        }
    }
}

impl<'arena, T: AsIoSlices<'arena>> IntoInner for SendImpl<'arena, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data and source address.
pub struct RecvFromImpl<'arena, T: AsIoSlicesMut<'arena>> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena, T: AsIoSlicesMut<'arena>> RecvFromImpl<'arena, T> {
    /// Create [`RecvFrom`] or [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
            _lifetime: PhantomData,
        }
    }

    pub(crate) fn set_msg(&mut self) {
        // SAFETY: IoSliceMut is Unpin
        let mut slices = unsafe { self.buffer.as_io_slices_mut() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: std::mem::size_of_val(&self.addr) as _,
            msg_iov: slices.as_mut_ptr() as _,
            msg_iovlen: slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
    }
}

impl<'arena, T: AsIoSlicesMut<'arena>> IntoInner for RecvFromImpl<'arena, T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

/// Send data to specified address.
pub struct SendToImpl<'arena, T: AsIoSlices<'arena>> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) msg: libc::msghdr,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena, T: AsIoSlices<'arena>> SendToImpl<'arena, T> {
    /// Create [`SendTo`] or [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            msg: unsafe { std::mem::zeroed() },
            _lifetime: PhantomData,
        }
    }

    pub(crate) fn set_msg(&mut self) {
        // SAFETY: IoSlice is Unpin
        let slices = unsafe { self.buffer.as_io_slices() };
        self.msg = libc::msghdr {
            msg_name: self.addr.as_ptr() as _,
            msg_namelen: self.addr.len(),
            msg_iov: slices.as_ptr() as _,
            msg_iovlen: slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
    }
}

impl<'arena, T: AsIoSlices<'arena>> IntoInner for SendToImpl<'arena, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
