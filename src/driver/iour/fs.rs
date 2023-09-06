use std::{io, path::Path};

use crate::{
    driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    fs::File,
};

pub fn file_with_options(
    path: impl AsRef<Path>,
    options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    options.open(path)
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            inner: std::fs::File::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}
