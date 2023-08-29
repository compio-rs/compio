use crate::driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{io, path::Path};

pub struct FileInner {
    file: std::fs::File,
}

impl FileInner {
    pub fn with_options(path: impl AsRef<Path>, options: std::fs::OpenOptions) -> io::Result<Self> {
        Ok(Self {
            file: options.open(path)?,
        })
    }
}

impl AsRawFd for FileInner {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl FromRawFd for FileInner {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            file: std::fs::File::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for FileInner {
    fn into_raw_fd(self) -> RawFd {
        self.file.into_raw_fd()
    }
}
