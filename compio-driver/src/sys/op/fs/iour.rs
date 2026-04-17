use io_uring::{opcode, types::*};
use rustix::fs::{self, OFlags};

use crate::{IourOpCode as OpCode, OpEntry, sys::op::*};

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::OpenAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .flags(self.flags.union(OFlags::CLOEXEC).bits() as _)
            .mode(self.mode.bits())
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, res: &io::Result<usize>, _: &Extra) {
        if let Ok(fd) = res {
            // SAFETY: fd is a valid fd returned from kernel
            let fd = unsafe { OwnedFd::from_raw_fd(*fd as _) };
            self.opened_fd = Some(fd);
        }
    }
}

unsafe impl OpCode for CloseFile {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Close::new(Fd(self.fd.as_fd().as_raw_fd()))
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Ftruncate::new(Fd(self.fd.as_fd().as_raw_fd()), self.size)
            .build()
            .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        self.call()
    }
}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
    pub(crate) stat: fs::Statx,
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

unsafe impl<S: AsFd> OpCode for FileStat<S> {
    type Control = ();

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        static EMPTY_NAME: &[u8] = b"\0";
        opcode::Statx::new(
            Fd(self.fd.as_fd().as_fd().as_raw_fd()),
            EMPTY_NAME.as_ptr().cast(),
            &raw mut self.stat as _,
        )
        .flags(libc::AT_EMPTY_PATH)
        .mask(STATX_MASK.bits())
        .build()
        .into()
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        self.stat = pal::statx(self.fd.as_fd(), c"", false)?;

        Ok(0)
    }
}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

/// Get metadata from path.
pub struct PathStat<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) stat: fs::Statx,
    pub(crate) follow_symlink: bool,
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

unsafe impl<S: AsFd> OpCode for PathStat<S> {
    type Control = ();

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        opcode::Statx::new(
            Fd(self.dirfd.as_fd().as_raw_fd()),
            self.path.as_ptr(),
            &raw mut self.stat as _,
        )
        .flags(flags)
        .mask(STATX_MASK.bits())
        .build()
        .into()
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        self.stat = statx(self.dirfd.as_fd(), &self.path, self.follow_symlink)?;

        Ok(0)
    }
}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Fsync::new(Fd(self.fd.as_fd().as_raw_fd()))
            .flags(if self.datasync {
                FsyncFlags::DATASYNC
            } else {
                FsyncFlags::empty()
            })
            .build()
            .into()
    }
}

unsafe impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::UnlinkAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .flags(if self.dir { libc::AT_REMOVEDIR } else { 0 })
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::MkDirAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .mode(self.mode.bits())
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::RenameAt::new(
            Fd(self.old_dirfd.as_fd().as_raw_fd()),
            self.old_path.as_ptr(),
            Fd(self.new_dirfd.as_fd().as_raw_fd()),
            self.new_path.as_ptr(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::SymlinkAt::new(
            Fd(self.dirfd.as_fd().as_raw_fd()),
            self.source.as_ptr(),
            self.target.as_ptr(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::LinkAt::new(
            Fd(self.source_dirfd.as_fd().as_raw_fd()),
            self.source.as_ptr(),
            Fd(self.target_dirfd.as_fd().as_raw_fd()),
            self.target.as_ptr(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Splice::new(
            Fd(self.fd_in.as_fd().as_raw_fd()),
            self.offset_in,
            Fd(self.fd_out.as_fd().as_raw_fd()),
            self.offset_out,
            self.len.try_into().unwrap_or(u32::MAX),
        )
        .flags(self.flags.bits())
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}
