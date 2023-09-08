use std::io::{IoSlice, IoSliceMut};

use io_uring::{
    opcode,
    squeue::Entry,
    types::{Fixed, FsyncFlags},
};
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IntoInner, IoBuf, IoBufMut, OneOrVec},
    driver::{OpCode, RegisteredFd},
    op::*,
};

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn create_entry(&mut self) -> Entry {
        let slice = self.buffer.as_uninit_slice();
        opcode::Read::new(
            Fixed(u32::from(self.fd)),
            slice.as_mut_ptr() as _,
            slice.len() as _,
        )
        .offset(self.offset as _)
        .build()
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn create_entry(&mut self) -> Entry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fixed(u32::from(self.fd)), slice.as_ptr(), slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl OpCode for Sync {
    fn create_entry(&mut self) -> Entry {
        opcode::Fsync::new(Fixed(u32::from(self.fd)))
            .flags(if self.datasync {
                FsyncFlags::DATASYNC
            } else {
                FsyncFlags::empty()
            })
            .build()
    }
}

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: RegisteredFd,
    pub(crate) buffer: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
}

impl Accept {
    /// Create [`Accept`].
    pub fn new(fd: RegisteredFd) -> Self {
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

impl OpCode for Accept {
    fn create_entry(&mut self) -> Entry {
        opcode::Accept::new(
            Fixed(u32::from(self.fd)),
            &mut self.buffer as *mut sockaddr_storage as *mut libc::sockaddr,
            &mut self.addr_len,
        )
        .build()
    }
}

impl OpCode for Connect {
    fn create_entry(&mut self) -> Entry {
        opcode::Connect::new(
            Fixed(u32::from(self.fd)),
            self.addr.as_ptr(),
            self.addr.len(),
        )
        .build()
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvImpl<T> {
    fn create_entry(&mut self) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        opcode::Readv::new(
            Fixed(u32::from(self.fd)),
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
            Fixed(u32::from(self.fd)),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
    }
}

/// Receive data and source address.
pub struct RecvFromImpl<T: AsIoSlicesMut> {
    pub(crate) fd: RegisteredFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) slices: OneOrVec<IoSliceMut<'static>>,
    msg: libc::msghdr,
}

impl<T: AsIoSlicesMut> RecvFromImpl<T> {
    /// Create [`RecvFrom`] or [`RecvFromVectored`].
    pub fn new(fd: RegisteredFd, buffer: T::Inner) -> Self {
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
        opcode::RecvMsg::new(Fixed(u32::from(self.fd)), &mut self.msg).build()
    }
}

/// Send data to specified address.
pub struct SendToImpl<T: AsIoSlices> {
    pub(crate) fd: RegisteredFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) slices: OneOrVec<IoSlice<'static>>,
    msg: libc::msghdr,
}

impl<T: AsIoSlices> SendToImpl<T> {
    /// Create [`SendTo`] or [`SendToVectored`].
    pub fn new(fd: RegisteredFd, buffer: T::Inner, addr: SockAddr) -> Self {
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
        opcode::SendMsg::new(Fixed(u32::from(self.fd)), &self.msg).build()
    }
}
