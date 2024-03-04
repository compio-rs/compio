use std::{io, path::Path};

use compio_driver::{op::OpenFile, FromRawFd, RawFd};
use compio_runtime::Runtime;

use crate::{path_string, File};

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

    pub async fn open(&self, p: impl AsRef<Path>) -> io::Result<File> {
        let mut flags = libc::O_CLOEXEC
            | self.get_access_mode()?
            | self.get_creation_mode()?
            | (self.custom_flags as libc::c_int & !libc::O_ACCMODE);
        // Don't set nonblocking with epoll.
        if cfg!(not(any(target_os = "linux", target_os = "android"))) {
            flags |= libc::O_NONBLOCK;
        }
        let p = path_string(p)?;
        let op = OpenFile::new(p, flags, self.mode);
        let fd = Runtime::current().submit(op).await.0? as RawFd;
        File::new(unsafe { std::fs::File::from_raw_fd(fd) })
    }
}
