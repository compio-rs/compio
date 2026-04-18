//! OpCodes shared by both iour & polling driver

use rustix::fs::*;

use crate::op::*;

/// Open or create a file with flags and mode.
pub struct OpenFile<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) flags: OFlags,
    pub(crate) mode: Mode,
    pub(crate) opened_fd: Option<OwnedFd>,
}

impl<S: AsFd> OpenFile<S> {
    /// Create [`OpenFile`].
    pub fn new(dirfd: S, path: CString, flags: OFlags, mode: Mode) -> Self {
        Self {
            dirfd,
            path,
            flags,
            mode,
            opened_fd: None,
        }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        self.opened_fd = Some(openat(
            self.dirfd.as_fd(),
            &self.path,
            self.flags | OFlags::CLOEXEC,
            self.mode,
        )?);
        Ok(0)
    }
}

impl<S: AsFd> IntoInner for OpenFile<S> {
    type Inner = OwnedFd;

    fn into_inner(self) -> Self::Inner {
        self.opened_fd.expect("file not opened")
    }
}

impl CloseFile {
    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        unsafe { ManuallyDrop::drop(&mut self.fd) };
        Ok(0)
    }
}

/// Truncates or extends the underlying file, updating the size of file `S` to
/// `size`.
#[derive(Debug)]
pub struct TruncateFile<S: AsFd> {
    pub(crate) fd: S,
    pub(crate) size: u64,
}

impl<S: AsFd> TruncateFile<S> {
    /// Create [`TruncateFile`].
    pub fn new(fd: S, size: u64) -> Self {
        Self { fd, size }
    }

    pub(crate) fn call(&self) -> io::Result<usize> {
        ftruncate(self.fd.as_fd(), self.size)?;
        Ok(0)
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct VectoredControl {
    pub(crate) slices: Vec<SysSlice>,
}

/// Read a file at specified position into vectored buffer.
pub struct ReadVectoredAt<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
}

impl<T: IoVectoredBufMut, S> ReadVectoredAt<T, S> {
    /// Create [`ReadVectoredAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self { fd, offset, buffer }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for ReadVectoredAt<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from vectored buffer.
pub struct WriteVectoredAt<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
}

impl<T: IoVectoredBuf, S> WriteVectoredAt<T, S> {
    /// Create [`WriteVectoredAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self { fd, offset, buffer }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for WriteVectoredAt<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive a file into vectored buffer.
pub struct ReadVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
}

impl<T: IoVectoredBufMut, S> ReadVectored<T, S> {
    /// Create [`ReadVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for ReadVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send to a file from vectored buffer.
pub struct WriteVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
}

impl<T: IoVectoredBuf, S> WriteVectored<T, S> {
    /// Create [`WriteVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self { fd, buffer }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for WriteVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Remove file or directory.
pub struct Unlink<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) dir: bool,
}

impl<S: AsFd> Unlink<S> {
    /// Create [`Unlink`].
    pub fn new(dirfd: S, path: CString, dir: bool) -> Self {
        Self { dirfd, path, dir }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        let flags = if self.dir {
            AtFlags::REMOVEDIR
        } else {
            AtFlags::empty()
        };

        unlinkat(self.dirfd.as_fd(), &self.path, flags)?;

        Ok(0)
    }
}

/// Create a directory.
pub struct CreateDir<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) mode: Mode,
}

impl<S: AsFd> CreateDir<S> {
    /// Create [`CreateDir`].
    pub fn new(dirfd: S, path: CString, mode: Mode) -> Self {
        Self { dirfd, path, mode }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        mkdirat(self.dirfd.as_fd(), &self.path, self.mode)?;

        Ok(0)
    }
}

/// Rename a file or directory.
pub struct Rename<S1: AsFd, S2: AsFd> {
    pub(crate) old_dirfd: S1,
    pub(crate) old_path: CString,
    pub(crate) new_dirfd: S2,
    pub(crate) new_path: CString,
}

impl<S1: AsFd, S2: AsFd> Rename<S1, S2> {
    /// Create [`Rename`].
    pub fn new(old_dirfd: S1, old_path: CString, new_dirfd: S2, new_path: CString) -> Self {
        Self {
            old_dirfd,
            old_path,
            new_dirfd,
            new_path,
        }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        renameat(
            self.old_dirfd.as_fd(),
            &self.old_path,
            self.new_dirfd.as_fd(),
            &self.new_path,
        )?;

        Ok(0)
    }
}

/// Create a symlink.
pub struct Symlink<S: AsFd> {
    pub(crate) source: CString,
    pub(crate) dirfd: S,
    pub(crate) target: CString,
}

impl<S: AsFd> Symlink<S> {
    /// Create [`Symlink`]. `target` is a symlink to `source`.
    pub fn new(source: CString, dirfd: S, target: CString) -> Self {
        Self {
            source,
            dirfd,
            target,
        }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        symlinkat(&self.source, self.dirfd.as_fd(), &self.target)?;

        Ok(0)
    }
}

/// Create a hard link.
pub struct HardLink<S1: AsFd, S2: AsFd> {
    pub(crate) source_dirfd: S1,
    pub(crate) source: CString,
    pub(crate) target_dirfd: S2,
    pub(crate) target: CString,
}

impl<S1: AsFd, S2: AsFd> HardLink<S1, S2> {
    /// Create [`HardLink`]. `target` is a hard link to `source`.
    pub fn new(source_dirfd: S1, source: CString, target_dirfd: S2, target: CString) -> Self {
        Self {
            source_dirfd,
            source,
            target_dirfd,
            target,
        }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        linkat(
            self.source_dirfd.as_fd(),
            &self.source,
            self.target_dirfd.as_fd(),
            &self.target,
            AtFlags::empty(),
        )?;
        Ok(0)
    }
}

/// Poll a file descriptor for specified [`Interest`].
pub struct PollOnce<S> {
    pub(crate) fd: S,
    pub(crate) interest: Interest,
}

impl<S> PollOnce<S> {
    /// Create [`PollOnce`].
    pub fn new(fd: S, interest: Interest) -> Self {
        Self { fd, interest }
    }
}

impl<S> IntoInner for PollOnce<S> {
    type Inner = S;

    fn into_inner(self) -> Self::Inner {
        self.fd
    }
}

/// Create a pipe.
pub struct Pipe {
    pub(crate) fds: [Option<OwnedFd>; 2],
}

// Niche optimization.
const _: () = assert!(std::mem::size_of::<Option<OwnedFd>>() == std::mem::size_of::<RawFd>());

impl Pipe {
    /// Create [`Pipe`].
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self { fds: [None, None] }
    }

    pub(crate) fn call(&mut self) -> io::Result<usize> {
        self.fds = mk_pipe()?;
        Ok(0)
    }
}

impl IntoInner for Pipe {
    type Inner = (OwnedFd, OwnedFd);

    fn into_inner(self) -> Self::Inner {
        let [read_fd, write_fd] = self.fds;
        let read_fd = read_fd.expect("pipe not created");
        let write_fd = write_fd.expect("pipe not created");
        (read_fd, write_fd)
    }
}
