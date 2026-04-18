use std::{io, os::fd::AsFd, path::Path};

use compio_buf::{IntoInner, buf_try};
#[cfg(dirfd)]
use compio_driver::ToSharedFd;
use compio_driver::op::{CurrentDir, Mode, OFlags, OpenFile};
use rustix::io::{Errno, Result};

use crate::{File, path_string};

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    custom_flags: OFlags,
    mode: Mode,
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions {
            read: false,
            write: false,
            truncate: false,
            create: false,
            create_new: false,
            custom_flags: OFlags::empty(),
            mode: Mode::from_bits_retain(0o666),
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
        self.custom_flags = OFlags::from_bits_retain(flags as _).difference(OFlags::ACCMODE);
    }

    pub fn mode(&mut self, mode: u32) {
        self.mode = Mode::from_bits_retain(mode as _);
    }

    fn get_access_mode(&self) -> Result<OFlags> {
        match (self.read, self.write) {
            (true, false) => Ok(OFlags::RDONLY),
            (false, true) => Ok(OFlags::WRONLY),
            (true, true) => Ok(OFlags::RDWR),
            (false, false) => Err(Errno::INVAL),
        }
    }

    fn get_creation_mode(&self) -> Result<OFlags> {
        if !self.write && (self.truncate || self.create || self.create_new) {
            return Err(Errno::INVAL);
        }

        Ok(match (self.create, self.truncate, self.create_new) {
            (false, false, false) => OFlags::empty(),
            (true, false, false) => OFlags::CREATE,
            (false, true, false) => OFlags::TRUNC,
            (true, true, false) => OFlags::CREATE | OFlags::TRUNC,
            (_, _, true) => OFlags::CREATE | OFlags::EXCL,
        })
    }

    async fn open_impl(&self, dir: impl AsFd + 'static, p: impl AsRef<Path>) -> io::Result<File> {
        let flags = OFlags::CLOEXEC
            | self.get_access_mode()?
            | self.get_creation_mode()?
            | self.custom_flags;
        let p = path_string(p)?;
        let op = OpenFile::new(dir, p, flags, self.mode);
        let (_, op) = buf_try!(@try compio_runtime::submit(op).await);
        File::from_std(op.into_inner().into())
    }

    pub async fn open(&self, p: impl AsRef<Path>) -> io::Result<File> {
        self.open_impl(CurrentDir, p).await
    }

    #[cfg(dirfd)]
    pub async fn open_at(&self, dir: &File, path: impl AsRef<Path>) -> io::Result<File> {
        self.open_impl(dir.to_shared_fd(), path).await
    }
}
