use std::{
    io::{IoSlice, IoSliceMut},
    os::fd::RawFd,
    pin::Pin,
};

use compio_buf::IntoInner;
use io_uring::{
    opcode,
    squeue::Entry,
    types::{Fd, FsyncFlags},
};
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

pub use crate::driver::unix::op::*;
use crate::{
    buf::{IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut},
    driver::OpCode,
    op::*,
};

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let fd = Fd(self.fd);
        let slice = self.buffer.as_uninit_slice();
        opcode::Read::new(fd, slice.as_mut_ptr() as _, slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _)
            .offset(self.offset as _)
            .build()
    }
}

impl OpCode for Sync {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        opcode::Fsync::new(Fd(self.fd))
            .flags(if self.datasync {
                FsyncFlags::DATASYNC
            } else {
                FsyncFlags::empty()
            })
            .build()
    }
}

impl OpCode for Accept {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        opcode::Accept::new(
            Fd(self.fd),
            &mut self.buffer as *mut sockaddr_storage as *mut libc::sockaddr,
            &mut self.addr_len,
        )
        .build()
    }
}

impl OpCode for Connect {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        opcode::Connect::new(Fd(self.fd), self.addr.as_ptr(), self.addr.len()).build()
    }
}

impl<T: IoBufMut> OpCode for Recv<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let fd = self.fd;
        let slice = self.buffer.as_uninit_slice();
        opcode::Read::new(Fd(fd), slice.as_mut_ptr() as _, slice.len() as _).build()
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        opcode::Readv::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    fn create_entry(self: Pin<&mut Self>) -> Entry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _).build()
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        self.slices = unsafe { self.buffer.as_io_slices() };
        opcode::Writev::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
    }
}

struct RecvFromImpl<T> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
}

impl<T> RecvFromImpl<T> {
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
        }
    }

    pub fn create_entry(&mut self, slices: &mut [IoSliceMut]) -> Entry {
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: std::mem::size_of_val(&self.addr) as _,
            msg_iov: slices.as_mut_ptr() as _,
            msg_iovlen: slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::RecvMsg::new(Fd(self.fd), &mut self.msg).build()
    }
}

impl<T> IntoInner for RecvFromImpl<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    base: RecvFromImpl<T>,
    slice: [IoSliceMut<'static>; 1],
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            base: RecvFromImpl::new(fd, buffer),
            slice: [IoSliceMut::new(&mut [])],
        }
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let this = &mut *self;
        this.slice[0] = unsafe { this.base.buffer.as_io_slice_mut() };
        this.base.create_entry(&mut this.slice)
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        self.base.into_inner()
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut> {
    base: RecvFromImpl<T>,
    slice: Vec<IoSliceMut<'static>>,
}

impl<T: IoVectoredBufMut> RecvFromVectored<T> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            base: RecvFromImpl::new(fd, buffer),
            slice: vec![],
        }
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let this = &mut *self;
        this.slice = unsafe { this.base.buffer.as_io_slices_mut() };
        this.base.create_entry(&mut this.slice)
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvFromVectored<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        self.base.into_inner()
    }
}

struct SendToImpl<T> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) msg: libc::msghdr,
}

impl<T> SendToImpl<T> {
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            msg: unsafe { std::mem::zeroed() },
        }
    }

    pub fn create_entry(&mut self, slices: &mut [IoSlice]) -> Entry {
        self.msg = libc::msghdr {
            msg_name: self.addr.as_ptr() as _,
            msg_namelen: self.addr.len(),
            msg_iov: slices.as_mut_ptr() as _,
            msg_iovlen: slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::SendMsg::new(Fd(self.fd), &self.msg).build()
    }
}

impl<T> IntoInner for SendToImpl<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    base: SendToImpl<T>,
    slice: [IoSlice<'static>; 1],
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            base: SendToImpl::new(fd, buffer, addr),
            slice: [IoSlice::new(&[])],
        }
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let this = &mut *self;
        this.slice[0] = unsafe { this.base.buffer.as_io_slice() };
        this.base.create_entry(&mut this.slice)
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.base.into_inner()
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf> {
    base: SendToImpl<T>,
    slice: Vec<IoSlice<'static>>,
}

impl<T: IoVectoredBuf> SendToVectored<T> {
    /// Create [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            base: SendToImpl::new(fd, buffer, addr),
            slice: vec![],
        }
    }
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> Entry {
        let this = &mut *self;
        this.slice = unsafe { this.base.buffer.as_io_slices() };
        this.base.create_entry(&mut this.slice)
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.base.into_inner()
    }
}
