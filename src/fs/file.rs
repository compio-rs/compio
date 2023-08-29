use crate::{
    driver::{fs::FileInner, AsRawFd, FromRawFd, IntoRawFd, RawFd},
    fs::OpenOptions,
};
use std::{io, path::Path};

pub struct File {
    inner: FileInner,
}

impl File {
    pub fn with_options(path: impl AsRef<Path>, options: OpenOptions) -> io::Result<Self> {
        Ok(Self {
            inner: FileInner::with_options(path, options.0)?,
        })
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            inner: FileInner::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}
