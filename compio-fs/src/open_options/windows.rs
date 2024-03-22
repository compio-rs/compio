use std::{io, os::windows::fs::OpenOptionsExt, path::Path};

use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

use crate::File;

#[derive(Clone, Debug)]
pub struct OpenOptions {
    opt: std::fs::OpenOptions,
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            opt: std::fs::OpenOptions::new(),
        }
    }

    pub fn read(&mut self, read: bool) {
        self.opt.read(read);
    }

    pub fn write(&mut self, write: bool) {
        self.opt.write(write);
    }

    pub fn truncate(&mut self, truncate: bool) {
        self.opt.truncate(truncate);
    }

    pub fn create(&mut self, create: bool) {
        self.opt.create(create);
    }

    pub fn create_new(&mut self, create_new: bool) {
        self.opt.create_new(create_new);
    }

    pub fn custom_flags(&mut self, flags: u32) {
        self.opt.custom_flags(flags);
    }

    pub fn access_mode(&mut self, access_mode: u32) {
        self.opt.access_mode(access_mode);
    }

    pub fn share_mode(&mut self, share_mode: u32) {
        self.opt.share_mode(share_mode);
    }

    pub fn attributes(&mut self, attrs: u32) {
        self.opt.attributes(attrs);
    }

    pub fn security_qos_flags(&mut self, flags: u32) {
        self.opt.security_qos_flags(flags);
    }

    pub async fn open(&self, p: impl AsRef<Path>) -> io::Result<File> {
        let mut opt = self.opt.clone();
        opt.attributes(FILE_FLAG_OVERLAPPED);
        let p = p.as_ref().to_path_buf();
        let file = compio_runtime::spawn_blocking(move || opt.open(p)).await?;
        File::new(file)
    }
}
