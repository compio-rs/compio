use std::io;

use mio::event::Event;

pub use crate::driver::unix_op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IoBuf, IoBufMut},
    driver::{Decision, OpCode},
    op::*,
};

/// Helper macro to execute a system call that returns an `io::Result`.
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { ::libc::$fn($($arg, )*) };
        if res == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res as _)
        }
    }};
}

/// Execute a system call, if would block, wait for it to be readable.
macro_rules! syscall_or_wait_writable {
    ($fn: ident ( $($arg: expr),* $(,)* ), $fd:expr) => {{
        match syscall!( $fn ( $($arg, )* )) {
            Ok(fd) => Ok(Decision::Completed(fd)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(Decision::wait_writable($fd)),
            Err(e) => Err(e),
        }
    }};
}

/// Execute a system call, if would block, wait for it to be writable.
macro_rules! syscall_or_wait_readable {
    ($fn: ident ( $($arg: expr),* $(,)* ), $fd:expr) => {{
        match syscall!( $fn ( $($arg, )* )) {
            Ok(fd) => Ok(Decision::Completed(fd)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Decision::wait_readable($fd)),
            Err(e) => Err(e),
        }
    }};
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_readable());

        let (ptr, len) = (self.buffer.as_buf_mut_ptr(), self.buffer.buf_len());
        syscall!(pread(self.fd, ptr as _, len, self.offset as _))
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_writable());

        let (ptr, len) = (self.buffer.as_buf_ptr(), self.buffer.buf_len());
        syscall!(pwrite(self.fd, ptr as _, len, self.offset as _))
    }
}

impl OpCode for Sync {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(fsync(self.fd))?))
    }

    fn on_event(&mut self, _: &Event) -> std::io::Result<usize> {
        unreachable!("Sync operation should not be submitted to mio")
    }
}

impl OpCode for Accept {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        syscall_or_wait_writable!(
            accept(
                self.fd,
                &mut self.buffer as *mut _ as *mut _,
                &mut self.addr_len
            ),
            self.fd
        )
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_readable());

        syscall!(accept(
            self.fd,
            &mut self.buffer as *mut _ as *mut _,
            &mut self.addr_len
        ))
    }
}

impl OpCode for Connect {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        syscall_or_wait_readable!(
            connect(self.fd, self.addr.as_ptr(), self.addr.len()),
            self.fd
        )
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_writable());

        syscall!(connect(self.fd, self.addr.as_ptr(), self.addr.len()))
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_readable());

        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        syscall!(readv(
            self.fd,
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        ))
    }
}

impl<T: AsIoSlices> OpCode for SendImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_writable());

        self.slices = unsafe { self.buffer.as_io_slices() };
        syscall!(writev(
            self.fd,
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        ))
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvFromImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: 128,
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        syscall_or_wait_readable!(recvmsg(self.fd, &mut self.msg, 0), self.fd)
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        debug_assert!(event.is_readable());

        syscall!(recvmsg(self.fd, &mut self.msg, 0))
    }
}

impl<T: AsIoSlices> OpCode for SendToImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        self.slices = unsafe { self.buffer.as_io_slices() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: 128,
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        syscall_or_wait_writable!(sendmsg(self.fd, &self.msg, 0), self.fd)
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        assert!(event.is_writable());

        syscall!(sendmsg(self.fd, &self.msg, 0))
    }
}
