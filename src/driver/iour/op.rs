use crate::{
    buf::{IoBuf, IoBufMut},
    driver::{OpCode, RawFd},
    op::{Connect, ReadAt, Recv, RecvFrom, Send, SendTo, WriteAt},
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

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(&mut self) -> Entry {
        self.fd;
        unimplemented!()
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    #[allow(clippy::no_effect)]
    fn create_entry(&mut self) -> Entry {
        let buffer = self.buffer.as_slice();
        opcode::SendZc::new(Fd(self.fd), buffer.as_ptr(), buffer.len() as _)
            .dest_addr(self.addr.as_ptr())
            .dest_addr_len(self.addr.len())
            .build()
    }
}
