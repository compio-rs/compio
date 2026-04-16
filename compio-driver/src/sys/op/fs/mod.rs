cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod iocp;
    } else if #[cfg(fusion)] {
        mod iour;
        mod poll;
        mod_use![fusion];
    } else if #[cfg(io_uring)] {
        mod_use![iour];
    } else if #[cfg(polling)] {
        mod_use![poll];
    } else if #[cfg(stub)] {
        mod_use![stub];
    }
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
    pub(crate) flags: u32,
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
        flags: u32,
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
}

#[cfg(linux_all)]
impl<S1, S2> IntoInner for Splice<S1, S2> {
    type Inner = (S1, S2);

    fn into_inner(self) -> Self::Inner {
        (self.fd_in, self.fd_out)
    }
}
