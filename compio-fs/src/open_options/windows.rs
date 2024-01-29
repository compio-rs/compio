use std::{io, path::Path, ptr::null};

use compio_buf::BufResult;
use compio_driver::{op::OpenFile, syscall, FromRawFd, RawFd};
use compio_runtime::Runtime;
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, ERROR_INVALID_PARAMETER, GENERIC_READ, GENERIC_WRITE},
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        FileAllocationInfo, SetFileInformationByHandle, CREATE_NEW, FILE_ALLOCATION_INFO,
        FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_OVERLAPPED,
        FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_ALWAYS,
        OPEN_EXISTING, SECURITY_SQOS_PRESENT, TRUNCATE_EXISTING,
    },
};

use crate::{path_string, File};

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    custom_flags: u32,
    access_mode: Option<u32>,
    attributes: FILE_FLAGS_AND_ATTRIBUTES,
    share_mode: FILE_SHARE_MODE,
    security_qos_flags: u32,
    security_attributes: *const SECURITY_ATTRIBUTES,
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
            access_mode: None,
            share_mode: FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            attributes: 0,
            security_qos_flags: 0,
            security_attributes: null(),
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

    pub unsafe fn security_attributes(&mut self, attrs: *const SECURITY_ATTRIBUTES) {
        self.security_attributes = attrs;
    }

    pub fn security_qos_flags(&mut self, flags: u32) {
        // We have to set `SECURITY_SQOS_PRESENT` here, because one of the valid flags
        // we can receive is `SECURITY_ANONYMOUS = 0x0`, which we can't check
        // for later on.
        self.security_qos_flags = flags | SECURITY_SQOS_PRESENT;
    }

    fn get_access_mode(&self) -> io::Result<u32> {
        match (self.read, self.write, self.access_mode) {
            (.., Some(mode)) => Ok(mode),
            (true, false, None) => Ok(GENERIC_READ),
            (false, true, None) => Ok(GENERIC_WRITE),
            (true, true, None) => Ok(GENERIC_READ | GENERIC_WRITE),
            (false, false, None) => Err(io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as _)),
        }
    }

    fn get_creation_mode(&self) -> io::Result<u32> {
        if !self.write && (self.truncate || self.create || self.create_new) {
            return Err(io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as _));
        }

        Ok(match (self.create, self.truncate, self.create_new) {
            (false, false, false) => OPEN_EXISTING,
            (true, false, false) => OPEN_ALWAYS,
            (false, true, false) => TRUNCATE_EXISTING,
            // https://github.com/rust-lang/rust/issues/115745
            (true, true, false) => OPEN_ALWAYS,
            (_, _, true) => CREATE_NEW,
        })
    }

    fn get_flags_and_attributes(&self) -> u32 {
        self.custom_flags
            | self.attributes
            | self.security_qos_flags
            | if self.create_new {
                FILE_FLAG_OPEN_REPARSE_POINT
            } else {
                0
            }
            | FILE_FLAG_OVERLAPPED
    }

    pub async fn open(&self, p: impl AsRef<Path>) -> io::Result<File> {
        let p = path_string(p)?;
        let creation_mode = self.get_creation_mode()?;
        let op = OpenFile::new(
            p,
            self.get_access_mode()?,
            self.share_mode,
            self.security_attributes,
            creation_mode,
            self.get_flags_and_attributes(),
        );
        let BufResult(fd, op) = Runtime::current().submit(op).await;
        let fd = fd? as RawFd;
        if self.truncate
            && creation_mode == OPEN_ALWAYS
            && op.last_os_error() == ERROR_ALREADY_EXISTS
        {
            let alloc = FILE_ALLOCATION_INFO { AllocationSize: 0 };
            syscall!(
                BOOL,
                SetFileInformationByHandle(
                    fd as _,
                    FileAllocationInfo,
                    std::ptr::addr_of!(alloc).cast(),
                    std::mem::size_of::<FILE_ALLOCATION_INFO>() as _,
                )
            )?;
        }
        File::new(unsafe { std::fs::File::from_raw_fd(fd) })
    }
}
