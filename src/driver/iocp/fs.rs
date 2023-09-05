use std::{
    io,
    os::windows::prelude::{
        AsRawHandle, FromRawHandle, IntoRawHandle, OpenOptionsExt, OwnedHandle,
    },
    path::Path,
};

use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

use crate::driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

pub struct FileInner {
    handle: OwnedHandle,
}

impl FileInner {
    pub fn with_options(
        path: impl AsRef<Path>,
        mut options: std::fs::OpenOptions,
    ) -> io::Result<Self> {
        options.custom_flags(FILE_FLAG_OVERLAPPED);
        let file = options.open(path)?;
        Ok(Self {
            handle: file.into(),
        })
    }
}

impl AsRawFd for FileInner {
    fn as_raw_fd(&self) -> RawFd {
        self.handle.as_raw_handle()
    }
}

impl FromRawFd for FileInner {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            handle: OwnedHandle::from_raw_handle(fd),
        }
    }
}

impl IntoRawFd for FileInner {
    fn into_raw_fd(self) -> RawFd {
        self.handle.into_raw_handle()
    }
}
