use std::{io, ops::ControlFlow};

use mio::event::Event;

pub use crate::driver::unix::op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IoBuf, IoBufMut},
    driver::{Decision, OpCode},
    op::*,
    syscall,
};

impl<'arena, T: IoBufMut<'arena>> OpCode for ReadAt<'arena, T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        if cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_os = "illumos"
        )) {
            let fd = self.fd;
            // SAFETY: slice into buffer is Unpin
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

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_readable());

        let fd = self.fd;
        // SAFETY: slice into buffer is Unpin
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

impl<'arena, T: IoBuf<'arena>> OpCode for WriteAt<'arena, T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        if cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_os = "illumos"
        )) {
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

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_writable());

        // SAFETY: buffer is Unpin
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
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::Completed(syscall!(fsync(self.fd))? as _))
    }

    fn on_event(&mut self, _: &Event) -> std::io::Result<ControlFlow<usize>> {
        unreachable!("Sync operation should not be submitted to mio")
    }
}

impl OpCode for Accept {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        // SAFETY: buffer is Unpin
        syscall!(
            accept(
                self.fd,
                &mut self.buffer as *mut _ as *mut _,
                &mut self.addr_len
            ) or wait_readable(self.fd)
        )
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_readable());

        match syscall!(accept(
            self.fd,
            &mut self.buffer as *mut _ as *mut _,
            &mut self.addr_len
        )) {
            Ok(fd) => Ok(ControlFlow::Break(fd as _)),
            Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => Ok(ControlFlow::Continue(())),
            Err(e) => Err(e),
        }
    }
}

impl OpCode for Connect {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        syscall!(
            connect(self.fd, self.addr.as_ptr(), self.addr.len()) or wait_writable(self.fd)
        )
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_writable());

        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(getsockopt(
            self.fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut err as *mut _ as *mut _,
            &mut err_len
        ))?;

        if err == 0 {
            Ok(ControlFlow::Break(0))
        } else {
            Err(io::Error::from_raw_os_error(err))
        }
    }
}

impl<'arena, T: AsIoSlicesMut<'arena>> OpCode for RecvImpl<'arena, T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_readable());

        let fd = self.fd;
        // SAFETY: IoSliceMut is Unpin
        let slices = unsafe { self.buffer.as_io_slices_mut() };
        syscall!(break readv(fd, slices.as_mut_ptr() as _, slices.len() as _,))
    }
}

impl<'arena, T: AsIoSlices<'arena>> OpCode for SendImpl<'arena, T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_writable());

        // SAFETY: IoSlice is Unpin
        let slices = unsafe { self.buffer.as_io_slices() };
        syscall!(break writev(self.fd, slices.as_ptr() as _, slices.len() as _,))
    }
}

impl<'arena, T: AsIoSlicesMut<'arena>> OpCode for RecvFromImpl<'arena, T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        let fd = self.fd;
        let msg = self.set_msg();
        syscall!(recvmsg(fd, msg, 0) or wait_readable(fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_readable());

        syscall!(break recvmsg(self.fd, &mut self.msg, 0))
    }
}

impl<'arena, T: AsIoSlices<'arena>> OpCode for SendToImpl<'arena, T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        let fd = self.fd;
        let msg = self.set_msg();
        syscall!(sendmsg(fd, msg, 0) or wait_writable(fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<ControlFlow<usize>> {
        debug_assert!(event.is_writable());

        syscall!(break sendmsg(self.fd, &self.msg, 0))
    }
}
