use std::{io, os::fd::AsFd, path::Path};

use compio_buf::{IntoInner, buf_try};
use compio_driver::{
    ToSharedFd,
    op::{CurrentDir, OpenFile},
};

use crate::{File, path_string};

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    custom_flags: i32,
    mode: libc::mode_t,
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            read: false,
            write: false,
            truncate: false,
            create: false,
            create_new: false,
            custom_flags: 0,
            mode: 0o666,
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

    pub fn custom_flags(&mut self, flags: i32) {
        self.custom_flags = flags;
    }

    pub fn mode(&mut self, mode: u32) {
        self.mode = mode as libc::mode_t;
    }

    fn get_access_mode(&self) -> io::Result<libc::c_int> {
        match (self.read, self.write) {
            (true, false) => Ok(libc::O_RDONLY),
            (false, true) => Ok(libc::O_WRONLY),
            (true, true) => Ok(libc::O_RDWR),
            (false, false) => Err(io::Error::from_raw_os_error(libc::EINVAL)),
        }
    }

    fn get_creation_mode(&self) -> io::Result<libc::c_int> {
        if !self.write && (self.truncate || self.create || self.create_new) {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }

        Ok(match (self.create, self.truncate, self.create_new) {
            (false, false, false) => 0,
            (true, false, false) => libc::O_CREAT,
            (false, true, false) => libc::O_TRUNC,
            (true, true, false) => libc::O_CREAT | libc::O_TRUNC,
            (_, _, true) => libc::O_CREAT | libc::O_EXCL,
        })
    }

    async fn open_impl(&self, dir: impl AsFd + 'static, p: impl AsRef<Path>) -> io::Result<File> {
        let flags = libc::O_CLOEXEC
            | self.get_access_mode()?
            | self.get_creation_mode()?
            | (self.custom_flags & !libc::O_ACCMODE);
        let p = path_string(p)?;
        let op = OpenFile::new(dir, p, flags, self.mode);
        let (_, op) = buf_try!(@try compio_runtime::submit(op).await);
        File::from_std(op.into_inner().into())
    }

    pub async fn open(&self, p: impl AsRef<Path>) -> io::Result<File> {
        self.open_impl(CurrentDir, p).await
    }

    pub async fn open_at(&self, dir: &File, path: impl AsRef<Path>) -> io::Result<File> {
        self.open_impl(dir.to_shared_fd(), path).await
    }
}
