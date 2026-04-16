use io_uring::{opcode, types::*};

use crate::{
    IourOpCode as OpCode, OpEntry,
    sys::{op::*, prelude::*},
};

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::OpenAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .flags(self.flags | libc::O_CLOEXEC)
            .mode(self.mode)
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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
    pub(crate) stat: Statx,
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        static EMPTY_NAME: &[u8] = b"\0";
        opcode::Statx::new(
            Fd(self.fd.as_fd().as_fd().as_raw_fd()),
            EMPTY_NAME.as_ptr().cast(),
            &raw mut self.stat as _,
        )
        .flags(libc::AT_EMPTY_PATH)
        .mask(statx_mask())
        .build()
        .into()
    }

    #[cfg(gnulinux)]
    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        static EMPTY_NAME: &[u8] = b"\0";
        let res = syscall!(libc::statx(
            self.fd.as_fd().as_raw_fd(),
            EMPTY_NAME.as_ptr().cast(),
            libc::AT_EMPTY_PATH,
            statx_mask(),
            &raw mut self.stat as _
        ))?;
        Ok(res as _)
    }

    #[cfg(not(gnulinux))]
    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        let mut stat = unsafe { std::mem::zeroed() };
        let res = syscall!(libc::fstat(self.fd.as_fd().as_raw_fd(), &mut stat))?;
        self.stat = stat_to_statx(stat);
        Ok(res as _)
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
    pub(crate) stat: Statx,
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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
        .mask(statx_mask())
        .build()
        .into()
    }

    #[cfg(gnulinux)]
    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        let res = syscall!(libc::statx(
            self.dirfd.as_fd().as_raw_fd(),
            self.path.as_ptr(),
            flags,
            statx_mask(),
            &raw mut self.stat
        ))?;
        Ok(res as _)
    }

    #[cfg(not(gnulinux))]
    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        let mut stat = unsafe { std::mem::zeroed() };
        let res = syscall!(libc::fstatat(
            self.dirfd.as_fd().as_raw_fd(),
            self.path.as_ptr(),
            &mut stat,
            flags
        ))?;
        self.stat = stat_to_statx(stat);
        Ok(res as _)
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::MkDirAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .mode(self.mode)
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Splice::new(
            Fd(self.fd_in.as_fd().as_raw_fd()),
            self.offset_in,
            Fd(self.fd_out.as_fd().as_raw_fd()),
            self.offset_out,
            self.len.try_into().unwrap_or(u32::MAX),
        )
        .flags(self.flags)
        .build()
        .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
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
        Ok(syscall!(libc::splice(
            self.fd_in.as_fd().as_raw_fd(),
            offset_in_ptr,
            self.fd_out.as_fd().as_raw_fd(),
            offset_out_ptr,
            self.len,
            self.flags,
        ))? as _)
    }
}
