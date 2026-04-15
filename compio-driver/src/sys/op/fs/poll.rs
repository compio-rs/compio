use crate::{
    Decision, PollOpCode as OpCode,
    sys::{op::*, prelude::*},
};

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
    pub(crate) stat: Stat,
}

/// Get metadata from path.
pub struct PathStat<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) stat: Stat,
    pub(crate) follow_symlink: bool,
}

#[doc(hidden)]
pub struct SyncControl {
    #[allow(dead_code)]
    aiocb: Aiocb,
}

impl Default for SyncControl {
    fn default() -> Self {
        Self { aiocb: new_aiocb() }
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = SyncControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        _ = ctrl;
        #[cfg(aio)]
        {
            ctrl.aiocb.aio_fildes = self.fd.as_fd().as_raw_fd();
        }
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            unsafe extern "C" fn aio_fsync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_SYNC, aiocbp) }
            }
            unsafe extern "C" fn aio_fdatasync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_DSYNC, aiocbp) }
            }

            let f = if self.datasync {
                aio_fdatasync
            } else {
                aio_fsync
            };

            Ok(Decision::aio(&mut ctrl.aiocb, f))
        }
        #[cfg(not(aio))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(crate::OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(datasync)]
        {
            Poll::Ready(Ok(syscall!(if self.datasync {
                libc::fdatasync(self.fd.as_fd().as_raw_fd())
            } else {
                libc::fsync(self.fd.as_fd().as_raw_fd())
            })? as _))
        }
        #[cfg(not(datasync))]
        {
            Poll::Ready(Ok(syscall!(libc::fsync(self.fd.as_fd().as_raw_fd()))? as _))
        }
    }
}

unsafe impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.call(control)?;
        self.opened_fd = Some(unsafe { OwnedFd::from_raw_fd(fd as _) });
        Poll::Ready(Ok(fd))
    }
}

unsafe impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

impl<S> FileStat<S> {
    /// Create [`FileStat`].
    pub fn new(fd: S) -> Self {
        Self {
            fd,
            stat: unsafe { std::mem::zeroed() },
        }
    }
}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

unsafe impl<S: AsFd> OpCode for FileStat<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(gnulinux)]
        {
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            static EMPTY_NAME: &[u8] = b"\0";
            syscall!(libc::statx(
                self.fd.as_fd().as_raw_fd(),
                EMPTY_NAME.as_ptr().cast(),
                libc::AT_EMPTY_PATH,
                statx_mask(),
                &mut s
            ))?;
            self.stat = statx_to_stat(s);
            Poll::Ready(Ok(0))
        }
        #[cfg(not(gnulinux))]
        {
            Poll::Ready(Ok(
                syscall!(libc::fstat(self.fd.as_fd().as_raw_fd(), &raw mut self.stat))? as _,
            ))
        }
    }
}

impl<S: AsFd> PathStat<S> {
    /// Create [`PathStat`].
    pub fn new(dirfd: S, path: CString, follow_symlink: bool) -> Self {
        Self {
            dirfd,
            path,
            stat: unsafe { std::mem::zeroed() },
            follow_symlink,
        }
    }
}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

unsafe impl<S: AsFd> OpCode for PathStat<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(gnulinux)]
        let res = {
            let mut flags = libc::AT_EMPTY_PATH;
            if !self.follow_symlink {
                flags |= libc::AT_SYMLINK_NOFOLLOW;
            }
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            let res = syscall!(libc::statx(
                self.dirfd.as_fd().as_raw_fd(),
                self.path.as_ptr(),
                flags,
                statx_mask(),
                &mut s
            ))?;
            self.stat = statx_to_stat(s);
            res
        };
        // Some platforms don't support `AT_EMPTY_PATH`, so we have to use `fstat` when
        // the path is empty.
        #[cfg(not(gnulinux))]
        let res = if self.path.is_empty() {
            syscall!(libc::fstat(
                self.dirfd.as_fd().as_raw_fd(),
                &raw mut self.stat
            ))?
        } else {
            syscall!(libc::fstatat(
                self.dirfd.as_fd().as_raw_fd(),
                self.path.as_ptr(),
                &raw mut self.stat,
                if !self.follow_symlink {
                    libc::AT_SYMLINK_NOFOLLOW
                } else {
                    0
                }
            ))?
        };
        Poll::Ready(Ok(res as _))
    }
}

#[cfg(linux_all)]
unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        use crate::sys::WaitArg;

        Ok(Decision::wait_for_many([
            WaitArg::readable(self.fd_in.as_fd().as_raw_fd()),
            WaitArg::writable(self.fd_out.as_fd().as_raw_fd()),
        ]))
    }

    fn op_type(&mut self, _: &mut Self::Control) -> Option<crate::OpType> {
        Some(crate::OpType::multi_fd([
            self.fd_in.as_fd().as_raw_fd(),
            self.fd_out.as_fd().as_raw_fd(),
        ]))
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        let mut offset_in = self.offset_in;
        let mut offset_out = self.offset_out;
        let offset_in_ptr = if offset_in < 0 {
            std::ptr::null_mut()
        } else {
            &mut offset_in
        };
        let offset_out_ptr = if offset_out < 0 {
            std::ptr::null_mut()
        } else {
            &mut offset_out
        };
        syscall!(
            break libc::splice(
                self.fd_in.as_fd().as_raw_fd(),
                offset_in_ptr,
                self.fd_out.as_fd().as_raw_fd(),
                offset_out_ptr,
                self.len,
                self.flags | libc::SPLICE_F_NONBLOCK,
            )
        )
    }
}
