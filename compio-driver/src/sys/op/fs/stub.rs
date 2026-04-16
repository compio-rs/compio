use crate::{
    OpCode,
    sys::{op::*, prelude::*},
};

impl<S: AsFd> OpCode for Sync<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
}

impl<S> FileStat<S> {
    /// Create [`FileStat`].
    pub fn new(fd: S) -> Self {
        Self { fd }
    }
}

impl<S: AsFd> OpCode for FileStat<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

/// Get metadata from path.
pub struct PathStat<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) follow_symlink: bool,
}

impl<S: AsFd> PathStat<S> {
    /// Create [`PathStat`].
    pub fn new(dirfd: S, path: CString, follow_symlink: bool) -> Self {
        Self {
            dirfd,
            path,
            follow_symlink,
        }
    }
}

impl<S: AsFd> OpCode for PathStat<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}
