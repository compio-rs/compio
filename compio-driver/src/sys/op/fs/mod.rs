cfg_select! {
    windows => {
        mod iocp;
    }
    fusion => {
        mod iour;
        mod poll;
        mod_use![fusion];
    }
    io_uring => {
        mod_use![iour];
    }
    polling => {
        mod_use![poll];
    }
    stub => {
        mod_use![stub];
    }
    _ => {}
}

use crate::sys::prelude::*;

/// Close the file fd.
pub struct CloseFile {
    pub(crate) fd: ManuallyDrop<OwnedFd>,
}

impl CloseFile {
    /// Create [`CloseFile`].
    pub fn new(fd: OwnedFd) -> Self {
        Self {
            fd: ManuallyDrop::new(fd),
        }
    }
}

/// Read an extended attribute from a path, following the final symbolic link.
///
/// The result is the number of bytes written, or the required size when the
/// buffer has zero capacity. This operation does not update the buffer's
/// initialized length.
///
/// Uses native io-uring when supported; otherwise the syscall runs on the
/// driver's blocking pool.
#[cfg(linux_all)]
pub struct GetXattr<T: IoBufMut> {
    pub(crate) path: CString,
    pub(crate) name: CString,
    pub(crate) buffer: T,
}

#[cfg(linux_all)]
impl<T: IoBufMut> GetXattr<T> {
    /// Create [`GetXattr`], retaining the path, name, and buffer until completion.
    pub fn new(path: CString, name: CString, buffer: T) -> Self {
        Self { path, name, buffer }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        let slice = self.buffer.sys_slice_mut();
        // SAFETY: both C strings are owned by this operation and remain valid.
        // The exclusively borrowed buffer exposes its entire writable capacity,
        // including uninitialized bytes. A size query writes no bytes.
        syscall!(libc::getxattr(
            self.path.as_ptr(),
            self.name.as_ptr(),
            slice.ptr().cast(),
            slice.len(),
        ))
    }
}

#[cfg(linux_all)]
impl<T: IoBufMut> IntoInner for GetXattr<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Read an extended attribute from an open file using `fgetxattr` semantics.
///
/// The result is the number of bytes written, or the required size when the
/// buffer has zero capacity. This operation does not update the buffer's
/// initialized length.
///
/// Uses native io-uring when supported; otherwise the syscall runs on the
/// driver's blocking pool.
#[cfg(linux_all)]
pub struct FGetXattr<S: AsFd, T: IoBufMut> {
    pub(crate) fd: S,
    pub(crate) name: CString,
    pub(crate) buffer: T,
}

#[cfg(linux_all)]
impl<S: AsFd, T: IoBufMut> FGetXattr<S, T> {
    /// Create [`FGetXattr`], retaining the fd, name, and buffer until completion.
    pub fn new(fd: S, name: CString, buffer: T) -> Self {
        Self { fd, name, buffer }
    }

    pub(crate) fn call(&mut self, _: &mut ()) -> io::Result<usize> {
        let slice = self.buffer.sys_slice_mut();
        // SAFETY: the fd and C string are retained by this operation. The
        // exclusively borrowed buffer exposes its entire writable capacity,
        // including uninitialized bytes. A size query writes no bytes.
        syscall!(libc::fgetxattr(
            self.fd.as_fd().as_raw_fd(),
            self.name.as_ptr(),
            slice.ptr().cast(),
            slice.len(),
        ))
    }
}

#[cfg(linux_all)]
impl<S: AsFd, T: IoBufMut> IntoInner for FGetXattr<S, T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Sync data to the disk.
pub struct Sync<S> {
    pub(crate) fd: S,
    #[allow(dead_code)]
    pub(crate) datasync: bool,
}

impl<S> Sync<S> {
    /// Create [`Sync`].
    ///
    /// If `datasync` is `true`, the file metadata may not be synchronized.
    pub fn new(fd: S, datasync: bool) -> Self {
        Self { fd, datasync }
    }
}

/// Splice data between two file descriptors.
#[cfg(linux_all)]
pub struct Splice<S1, S2> {
    pub(crate) fd_in: S1,
    pub(crate) offset_in: i64,
    pub(crate) fd_out: S2,
    pub(crate) offset_out: i64,
    pub(crate) len: usize,
    pub(crate) flags: rustix::pipe::SpliceFlags,
}

#[cfg(linux_all)]
impl<S1, S2> Splice<S1, S2> {
    /// Create [`Splice`].
    ///
    /// `offset_in` and `offset_out` specify the offset to read from and write
    /// to. Use `-1` for pipe ends or to use/update the current file
    /// position.
    pub fn new(
        fd_in: S1,
        offset_in: i64,
        fd_out: S2,
        offset_out: i64,
        len: usize,
        flags: rustix::pipe::SpliceFlags,
    ) -> Self {
        Self {
            fd_in,
            offset_in,
            fd_out,
            offset_out,
            len,
            flags,
        }
    }

    pub(crate) fn call(&self, _: &mut ()) -> io::Result<usize>
    where
        S1: AsFd,
        S2: AsFd,
    {
        let off_in = self.offset_in;
        let off_out = self.offset_out;

        rustix::pipe::splice(
            &self.fd_in,
            (off_in >= 0).then_some(&mut (off_in as u64)),
            &self.fd_out,
            (off_out >= 0).then_some(&mut (off_out as u64)),
            self.len,
            self.flags,
        )
        .map_err(Into::into)
    }
}

#[cfg(linux_all)]
impl<S1, S2> IntoInner for Splice<S1, S2> {
    type Inner = (S1, S2);

    fn into_inner(self) -> Self::Inner {
        (self.fd_in, self.fd_out)
    }
}
