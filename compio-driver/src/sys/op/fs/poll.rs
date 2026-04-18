use rustix::fs;

use crate::{Decision, PollOpCode as OpCode, sys::op::*};

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

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = AioControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.init_fd(&self.fd);
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        ctrl.decide_sync(self.datasync)
    }

    fn op_type(&mut self, ctrl: &mut Self::Control) -> Option<crate::OpType> {
        ctrl.op_type()
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(datasync)]
        if self.datasync {
            fs::fdatasync(self.fd.as_fd())?
        } else {
            fs::fsync(self.fd.as_fd())?
        }

        #[cfg(not(datasync))]
        fs::fsync(self.fd.as_fd())?;

        Poll::Ready(Ok(0))
    }
}

unsafe impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl OpCode for CloseFile {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
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

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.stat = stat(self.fd.as_fd(), c"", false)?;

        Poll::Ready(Ok(0))
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

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.stat = stat(self.dirfd.as_fd(), &self.path, self.follow_symlink)?;
        Poll::Ready(Ok(0))
    }
}

#[cfg(linux_all)]
unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

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

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        poll_io(|| self.call(control))
    }
}
