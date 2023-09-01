use crate::{
    buf::{IntoInner, IoBuf, IoBufMut},
    driver::{OpCode, RawFd},
    op::{Connect, ReadAt, Recv, Send, WriteAt},
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

impl<T: IoBufMut> OpCode for Recv<T> {
    fn create_entry(&mut self) -> Entry {
        let buffer = self.buffer.as_uninit_slice();
        opcode::Recv::new(Fd(self.fd), buffer.as_ptr() as _, buffer.len() as _).build()
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    fn create_entry(&mut self) -> Entry {
        let buffer = self.buffer.as_slice();
        opcode::Send::new(Fd(self.fd), buffer.as_ptr(), buffer.len() as _).build()
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    slice: libc::iovec,
    msg: libc::msghdr,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            slice: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
        }
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(&mut self) -> Entry {
        let buffer = self.buffer.as_uninit_slice();
        self.slice = libc::iovec {
            iov_base: buffer.as_mut_ptr() as _,
            iov_len: buffer.len(),
        };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: 128,
            msg_iov: &mut self.slice,
            msg_iovlen: 1,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::RecvMsg::new(Fd(self.fd), &mut self.msg).build()
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    slice: libc::iovec,
    msg: libc::msghdr,
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            slice: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
        }
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(&mut self) -> Entry {
        let buffer = self.buffer.as_slice();
        self.slice = libc::iovec {
            iov_base: buffer.as_ptr() as *const _ as _,
            iov_len: buffer.len(),
        };
        self.msg = libc::msghdr {
            msg_name: self.addr.as_ptr() as _,
            msg_namelen: self.addr.len(),
            msg_iov: &mut self.slice,
            msg_iovlen: 1,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::SendMsg::new(Fd(self.fd), &self.msg).build()
    }
}
