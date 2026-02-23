use std::{io, os::windows::fs::OpenOptionsExt, panic::resume_unwind, path::Path};

use compio_driver::ToSharedFd;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

use crate::File;

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    custom_flags: u32,
    access_mode: Option<u32>,
    share_mode: u32,
    attributes: u32,
    security_qos_flags: u32,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
            read: false,
            write: false,
            truncate: false,
            create: false,
            create_new: false,
            custom_flags: 0,
            access_mode: None,
            share_mode: FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            attributes: 0,
            security_qos_flags: 0,
        }
    }

    pub fn read(&mut self, read: bool) {
        self.read = read;
    }

    pub fn write(&mut self, write: bool) {
        self.write = write;
    }

    pub fn truncate(&mut self, truncate: bool) {
        self.truncate = truncate;
    }

    pub fn create(&mut self, create: bool) {
        self.create = create;
    }

    pub fn create_new(&mut self, create_new: bool) {
        self.create_new = create_new;
    }

    pub fn custom_flags(&mut self, flags: u32) {
        self.custom_flags = flags;
    }

    pub fn access_mode(&mut self, access_mode: u32) {
        self.access_mode = Some(access_mode);
    }

    pub fn share_mode(&mut self, share_mode: u32) {
        self.share_mode = share_mode;
    }

    pub fn attributes(&mut self, attrs: u32) {
        self.attributes = attrs;
    }

    pub fn security_qos_flags(&mut self, flags: u32) {
        self.security_qos_flags = flags;
    }

    pub async fn open(&self, p: impl AsRef<Path>) -> io::Result<File> {
        let opt = std::fs::OpenOptions::from(self);
        let p = p.as_ref().to_path_buf();
        let file = compio_runtime::spawn_blocking(move || opt.open(p))
            .await
            .unwrap_or_else(|e| resume_unwind(e))?;
        File::from_std(file)
    }

    #[cfg(dirfd)]
    pub async fn open_at(&self, dir: &File, p: impl AsRef<Path>) -> io::Result<File> {
        let opt = cap_primitives::fs::OpenOptions::from(self);
        let p = p.as_ref().to_path_buf();
        let file = crate::spawn_blocking_with(dir.to_shared_fd(), move |dir| {
            cap_primitives::fs::open(dir, &p, &opt)
        })
        .await?;
        File::from_std(file)
    }
}

impl From<&OpenOptions> for std::fs::OpenOptions {
    fn from(value: &OpenOptions) -> Self {
        let mut opt = std::fs::OpenOptions::new();
        opt.read(value.read)
            .write(value.write)
            .truncate(value.truncate)
            .create(value.create)
            .create_new(value.create_new)
            .custom_flags(value.custom_flags | FILE_FLAG_OVERLAPPED)
            .share_mode(value.share_mode)
            .attributes(value.attributes)
            .security_qos_flags(value.security_qos_flags);
        if let Some(access_mode) = value.access_mode {
            opt.access_mode(access_mode);
        }
        opt
    }
}

#[cfg(dirfd)]
impl From<&OpenOptions> for cap_primitives::fs::OpenOptions {
    fn from(value: &OpenOptions) -> Self {
        use cap_primitives::fs::OpenOptionsExt;

        let mut opt = cap_primitives::fs::OpenOptions::new();
        opt.read(value.read)
            .write(value.write)
            .truncate(value.truncate)
            .create(value.create)
            .create_new(value.create_new)
            .custom_flags(value.custom_flags | FILE_FLAG_OVERLAPPED)
            .share_mode(value.share_mode)
            .attributes(value.attributes)
            .security_qos_flags(value.security_qos_flags);
        if let Some(access_mode) = value.access_mode {
            opt.access_mode(access_mode);
        }
        opt
    }
}
