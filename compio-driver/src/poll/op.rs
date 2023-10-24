use std::{io, pin::Pin, task::Poll};

use compio_buf::{
    IntoInner, IoBuf, IoBufMut, IoSlice, IoSliceMut, IoVectoredBuf, IoVectoredBufMut,
};
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
use libc::open;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use libc::open64 as open;
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "hurd")))]
use libc::{pread, preadv, pwrite, pwritev};
#[cfg(any(target_os = "linux", target_os = "android", target_os = "hurd"))]
use libc::{pread64 as pread, preadv64 as preadv, pwrite64 as pwrite, pwritev64 as pwritev};
use polling::Event;
use socket2::SockAddr;

use super::{sockaddr_storage, socklen_t, syscall, Decision, OpCode, RawFd};
use crate::op::*;
pub use crate::unix::op::*;

impl OpCode for OpenFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(open(
            self.path.as_ptr(),
            self.flags,
            self.mode as libc::c_int
        ))? as _))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("OpenFile operation should not be submitted to polling")
    }
}

impl OpCode for CloseFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(libc::close(self.fd))? as _))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("CloseFile operation should not be submitted to polling")
    }
}

impl<T: IoBufMut> ReadAt<T> {
    unsafe fn call(&mut self) -> libc::ssize_t {
        let fd = self.fd;
        let slice = self.buffer.as_mut_slice();
        pread(
            fd,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            self.offset as _,
        )
    }
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            Ok(Decision::Completed(syscall!(self.call())? as _))
        } else {
            Ok(Decision::wait_readable(self.fd))
        }
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl<T: IoVectoredBufMut> ReadVectoredAt<T> {
    unsafe fn call(&mut self) -> libc::ssize_t {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        preadv(
            self.fd,
            self.slices.as_ptr() as _,
            self.slices.len() as _,
            self.offset as _,
        )
    }
}

impl<T: IoVectoredBufMut> OpCode for ReadVectoredAt<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            Ok(Decision::Completed(syscall!(self.call())? as _))
        } else {
            Ok(Decision::wait_readable(self.fd))
        }
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl<T: IoBuf> WriteAt<T> {
    unsafe fn call(&self) -> libc::ssize_t {
        let slice = self.buffer.as_slice();
        pwrite(
            self.fd,
            slice.as_ptr() as _,
            slice.len() as _,
            self.offset as _,
        )
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            Ok(Decision::Completed(syscall!(self.call())? as _))
        } else {
            Ok(Decision::wait_writable(self.fd))
        }
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.call())
    }
}

impl<T: IoVectoredBuf> WriteVectoredAt<T> {
    unsafe fn call(&mut self) -> libc::ssize_t {
        self.slices = unsafe { self.buffer.as_io_slices() };
        pwritev(
            self.fd,
            self.slices.as_ptr() as _,
            self.slices.len() as _,
            self.offset as _,
        )
    }
}

impl<T: IoVectoredBuf> OpCode for WriteVectoredAt<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            Ok(Decision::Completed(syscall!(self.call())? as _))
        } else {
            Ok(Decision::wait_writable(self.fd))
        }
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.call())
    }
}

impl OpCode for Sync {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(libc::fsync(self.fd))? as _))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("Sync operation should not be submitted to polling")
    }
}

impl OpCode for ShutdownSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(
            syscall!(libc::shutdown(self.fd, self.how()))? as _,
        ))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("CreateSocket operation should not be submitted to polling")
    }
}

impl OpCode for CloseSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(libc::close(self.fd))? as _))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("CloseSocket operation should not be submitted to polling")
    }
}

impl Accept {
    unsafe fn call(&mut self) -> libc::c_int {
        libc::accept(
            self.fd,
            &mut self.buffer as *mut _ as *mut _,
            &mut self.addr_len,
        )
    }
}

impl OpCode for Accept {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(self.call(), wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl OpCode for Connect {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            libc::connect(self.fd, self.addr.as_ptr(), self.addr.len()),
            wait_writable(self.fd)
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(libc::getsockopt(
            self.fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut err as *mut _ as *mut _,
            &mut err_len
        ))?;

        let res = if err == 0 {
            Ok(0)
        } else {
            Err(io::Error::from_raw_os_error(err))
        };
        Poll::Ready(res)
    }
}

impl<T: IoBufMut> OpCode for Recv<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd;
        let slice = self.buffer.as_mut_slice();
        syscall!(break libc::read(fd, slice.as_mut_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        syscall!(break libc::readv(self.fd, self.slices.as_ptr() as _, self.slices.len() as _))
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(break libc::write(self.fd, slice.as_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        self.slices = unsafe { self.buffer.as_io_slices() };
        syscall!(break libc::writev(self.fd, self.slices.as_ptr() as _, self.slices.len() as _))
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
        }
    }

    unsafe fn call(&mut self) -> libc::ssize_t {
        let fd = self.fd;
        let slice = self.buffer.as_mut_slice();
        libc::recvfrom(
            fd,
            slice.as_mut_ptr() as _,
            slice.len(),
            0,
            &mut self.addr as *mut _ as _,
            &mut self.addr_len,
        )
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(self.call(), wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
}

impl<T: IoVectoredBufMut> RecvFromVectored<T> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
        }
    }

    fn set_msg(&mut self) {
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

    unsafe fn call(&mut self) -> libc::ssize_t {
        libc::recvmsg(self.fd, &mut self.msg, 0)
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.set_msg();
        syscall!(self.call(), wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvFromVectored<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self { fd, buffer, addr }
    }

    unsafe fn call(&self) -> libc::ssize_t {
        let slice = self.buffer.as_slice();
        libc::sendto(
            self.fd,
            slice.as_ptr() as _,
            slice.len(),
            0,
            self.addr.as_ptr(),
            self.addr.len(),
        )
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(self.call(), wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.call())
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) slices: Vec<IoSlice>,
    pub(crate) msg: libc::msghdr,
}

impl<T: IoVectoredBuf> SendToVectored<T> {
    /// Create [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            slices: vec![],
            msg: unsafe { std::mem::zeroed() },
        }
    }

    fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.as_io_slices() };
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

    unsafe fn call(&self) -> libc::ssize_t {
        libc::sendmsg(self.fd, &self.msg, 0)
    }
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.set_msg();
        syscall!(self.call(), wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.call())
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
