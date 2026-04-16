use crate::{
    Decision, OpType, PollOpCode as OpCode,
    sys::{op::*, prelude::*},
};

#[doc(hidden)]
pub struct ReadAtControl {
    #[allow(dead_code)]
    aiocb: Aiocb,
}

impl Default for ReadAtControl {
    fn default() -> Self {
        Self { aiocb: new_aiocb() }
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ReadAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        _ = ctrl;
        #[cfg(aio)]
        {
            let slice = self.buffer.sys_slice_mut();

            ctrl.aiocb.aio_fildes = self.fd.as_fd().as_raw_fd();
            ctrl.aiocb.aio_offset = self.offset as _;
            ctrl.aiocb.aio_buf = slice.ptr().cast();
            ctrl.aiocb.aio_nbytes = slice.len();
        }
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_read))
        }
        #[cfg(not(aio))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let offset = self.offset;
        let slice = self.buffer.sys_slice_mut();
        syscall!(break pread(fd, slice.ptr() as _, slice.len() as _, offset as _,))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = ReadVectoredAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_readv))
        }
        #[cfg(not(freebsd))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break preadv(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                self.offset as _,
            )
        )
    }
}

#[doc(hidden)]
pub struct WriteAtControl {
    #[allow(dead_code)]
    aiocb: Aiocb,
}

impl Default for WriteAtControl {
    fn default() -> Self {
        Self { aiocb: new_aiocb() }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = WriteAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        _ = ctrl;
        #[cfg(aio)]
        {
            let slice = self.buffer.as_init();

            ctrl.aiocb.aio_fildes = self.fd.as_fd().as_raw_fd();
            ctrl.aiocb.aio_offset = self.offset as _;
            ctrl.aiocb.aio_buf = slice.as_ptr() as _;
            ctrl.aiocb.aio_nbytes = slice.len();
        }
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_write))
        }
        #[cfg(not(aio))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        syscall!(
            break pwrite(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _,
            )
        )
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = WriteVectoredAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_writev))
        }
        #[cfg(not(freebsd))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break pwritev(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                self.offset as _,
            )
        )
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.buffer.sys_slice_mut();
        syscall!(break libc::read(fd, slice.ptr() as _, slice.len()))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = ReadVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break libc::readv(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _
            )
        )
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        syscall!(
            break libc::write(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len()
            )
        )
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = WriteVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break libc::writev(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _
            )
        )
    }
}

unsafe impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_for(
            self.fd.as_fd().as_raw_fd(),
            self.interest,
        ))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }
}

unsafe impl OpCode for Pipe {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(any(freebsd, solarish, linux_all))]
        {
            Poll::Ready(
                syscall!(libc::pipe2(
                    self.fds.as_mut_ptr().cast(),
                    libc::O_CLOEXEC | libc::O_NONBLOCK
                ))
                .map(|res| res as _),
            )
        }
        #[cfg(not(any(freebsd, solarish, linux_all)))]
        {
            use nix::fcntl::{F_GETFD, F_GETFL, F_SETFD, F_SETFL, FdFlag, OFlag, fcntl};

            syscall!(libc::pipe(self.fds.as_mut_ptr().cast()))?;
            let Some(f1) = self.fds[0].as_ref() else {
                unreachable!("pipe() succeeded but returned invalid fd")
            };
            let Some(f2) = self.fds[1].as_ref() else {
                unreachable!("pipe() succeeded but returned invalid fd")
            };

            fn set_cloexec(fd: &OwnedFd) -> nix::Result<()> {
                let flag = FdFlag::from_bits_retain(fcntl(fd, F_GETFD)?);
                fcntl(fd, F_SETFD(flag | FdFlag::FD_CLOEXEC))?;
                Ok(())
            }

            fn set_nonblock(fd: &OwnedFd) -> nix::Result<()> {
                let flag = OFlag::from_bits_retain(fcntl(fd, F_GETFL)?);
                fcntl(fd, F_SETFL(flag | OFlag::O_NONBLOCK))?;
                Ok(())
            }

            set_cloexec(f1)?;
            set_cloexec(f2)?;
            set_nonblock(f1)?;
            set_nonblock(f2)?;

            Poll::Ready(Ok(0))
        }
    }
}
