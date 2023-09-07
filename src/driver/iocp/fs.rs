use std::{
    io,
    os::windows::prelude::{AsRawHandle, FromRawHandle, IntoRawHandle, OpenOptionsExt},
    path::Path,
};

use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

use crate::{
    driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    fs::File,
};

pub fn file_with_options(
    path: impl AsRef<Path>,
    mut options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    options.custom_flags(FILE_FLAG_OVERLAPPED);
    options.open(path)
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_handle()
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            inner: std::fs::File::from_raw_handle(fd),
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_handle()
    }
}
