use std::io::{IoSlice, IoSliceMut};

use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IntoInner, IoBuf, IoBufMut, OneOrVec},
    driver::{OpCode, RawFd},
    op::{Connect, ReadAt, RecvImpl, SendImpl, WriteAt},
};
use io_uring::{opcode, squeue::Entry, types::Fd};
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn create_entry(&mut self) -> Entry {
        let slice = self.buffer.as_uninit_slice();
        opcode::Read::new(Fd(self.fd), slice.as_mut_ptr() as _, slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn create_entry(&mut self) -> Entry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

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
            addr_len: 0,
        }
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> SockAddr {
        unsafe { SockAddr::new(self.buffer, self.addr_len) }
    }
}

impl OpCode for Accept {
    fn create_entry(&mut self) -> Entry {
        opcode::Accept::new(
            Fd(self.fd),
            &mut self.buffer as *mut _ as *mut _,
            &mut self.addr_len,
        )
        .build()
    }
}

impl OpCode for Connect {
    fn create_entry(&mut self) -> Entry {
        opcode::Connect::new(Fd(self.fd), self.addr.as_ptr(), self.addr.len()).build()
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvImpl<T> {
    fn create_entry(&mut self) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        opcode::Readv::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
    }
}

impl<T: AsIoSlices> OpCode for SendImpl<T> {
    fn create_entry(&mut self) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices() };
        opcode::Writev::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
    }
}

/// Receive data and source address.
pub struct RecvFromImpl<T: AsIoSlicesMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) slices: OneOrVec<IoSliceMut<'static>>,
    msg: libc::msghdr,
}

impl<T: AsIoSlicesMut> RecvFromImpl<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T::Inner) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            addr: unsafe { std::mem::zeroed() },
            slices: OneOrVec::One(IoSliceMut::new(&mut [])),
            msg: unsafe { std::mem::zeroed() },
        }
    }
}

impl<T: AsIoSlicesMut> IntoInner for RecvFromImpl<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvFromImpl<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(&mut self) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: 128,
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len(),
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::RecvMsg::new(Fd(self.fd), &mut self.msg).build()
    }
}

/// Send data to specified address.
pub struct SendToImpl<T: AsIoSlices> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) slices: OneOrVec<IoSlice<'static>>,
    msg: libc::msghdr,
}

impl<T: AsIoSlices> SendToImpl<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T::Inner, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer: T::new(buffer),
            addr,
            slices: OneOrVec::One(IoSlice::new(&[])),
            msg: unsafe { std::mem::zeroed() },
        }
    }
}

impl<T: AsIoSlices> IntoInner for SendToImpl<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: AsIoSlices> OpCode for SendToImpl<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(&mut self) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices() };
        self.msg = libc::msghdr {
            msg_name: self.addr.as_ptr() as _,
            msg_namelen: self.addr.len(),
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len(),
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::SendMsg::new(Fd(self.fd), &self.msg).build()
    }
}
