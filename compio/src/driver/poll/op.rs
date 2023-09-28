use std::{
    io::{self, IoSlice, IoSliceMut},
    pin::Pin,
    task::Poll,
};

use compio_buf::IntoInner;
use polling::Event;
use socket2::SockAddr;

pub use crate::driver::unix::op::*;
use crate::{
    buf::{IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut},
    driver::{sockaddr_storage, socklen_t, Decision, OpCode, RawFd},
    op::*,
    syscall,
};

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            let fd = self.fd;
            let slice = self.buffer.as_uninit_slice();
            Ok(Decision::Completed(syscall!(pread(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                self.offset as _
            ))? as _))
        } else {
            Ok(Decision::wait_readable(self.fd))
        }
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd;
        let slice = self.buffer.as_uninit_slice();

        syscall!(
            break pread(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                self.offset as _
            )
        )
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            let slice = self.buffer.as_slice();
            Ok(Decision::Completed(syscall!(pwrite(
                self.fd,
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _
            ))? as _))
        } else {
            Ok(Decision::wait_writable(self.fd))
        }
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();

        syscall!(
            break pwrite(
                self.fd,
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _
            )
        )
    }
}

impl OpCode for Sync {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(fsync(self.fd))? as _))
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        unreachable!("Sync operation should not be submitted to polling")
    }
}

impl OpCode for Accept {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            accept(
                self.fd,
                &mut self.buffer as *mut _ as *mut _,
                &mut self.addr_len
            ) or wait_readable(self.fd)
        )
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(
            break accept(
                self.fd,
                &mut self.buffer as *mut _ as *mut _,
                &mut self.addr_len
            )
        )
    }
}

impl OpCode for Connect {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            connect(self.fd, self.addr.as_ptr(), self.addr.len()) or wait_writable(self.fd)
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(getsockopt(
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
        let slice = self.buffer.as_uninit_slice();
        syscall!(break read(fd, slice.as_mut_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        syscall!(break readv(self.fd, self.slices.as_ptr() as _, self.slices.len() as _))
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(break write(self.fd, slice.as_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        self.slices = unsafe { self.buffer.as_io_slices() };
        syscall!(break writev(self.fd, self.slices.as_ptr() as _, self.slices.len() as _))
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
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd;
        let slice = self.buffer.as_uninit_slice();
        syscall!(
            recvfrom(
                fd,
                slice.as_mut_ptr() as _,
                slice.len(),
                0,
                &mut self.addr as *mut _ as _,
                &mut self.addr_len
            ) or wait_readable(self.fd)
        )
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd;
        let slice = self.buffer.as_uninit_slice();
        syscall!(
            break recvfrom(
                fd,
                slice.as_mut_ptr() as _,
                slice.len(),
                0,
                &mut self.addr as *mut _ as _,
                &mut self.addr_len
            )
        )
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
    pub(crate) slices: Vec<IoSliceMut<'static>>,
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
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.set_msg();
        syscall!(recvmsg(self.fd, &mut self.msg, 0) or wait_readable(self.fd))
    }

    fn on_event(mut self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break recvmsg(self.fd, &mut self.msg, 0))
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
}

impl<T: IoBuf> OpCode for SendTo<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let slice = self.buffer.as_slice();
        syscall!(
            sendto(
                self.fd,
                slice.as_ptr() as _,
                slice.len(),
                0,
                self.addr.as_ptr(),
                self.addr.len()
            ) or wait_writable(self.fd)
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(
            break sendto(
                self.fd,
                slice.as_ptr() as _,
                slice.len(),
                0,
                self.addr.as_ptr(),
                self.addr.len()
            )
        )
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
    pub(crate) slices: Vec<IoSlice<'static>>,
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
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.set_msg();
        syscall!(sendmsg(self.fd, &self.msg, 0) or wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break sendmsg(self.fd, &self.msg, 0))
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
