use std::{
    io,
    path::Path,
    time::{Duration, SystemTime},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    op::{FileMetadata, PathStat},
    syscall,
};
use compio_runtime::Runtime;
use windows_sys::Win32::Storage::FileSystem::{
    SetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT,
};

use crate::path_string;

async fn metadata_impl(path: impl AsRef<Path>, follow_symlink: bool) -> io::Result<Metadata> {
    let path = path_string(path)?;
    let op = PathStat::new(path, follow_symlink);
    let BufResult(res, op) = Runtime::current().submit(op).await;
    res.map(|_| Metadata::from_stat(op.into_inner()))
}

pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(path, true).await
}

pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(path, false).await
}

pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path_string(path)?;
    Runtime::current()
        .spawn_blocking(move || {
            syscall!(BOOL, SetFileAttributesW(path.as_ptr(), perm.attrs))?;
            Ok(())
        })
        .await
}

#[inline]
fn filetime_to_systemtime(tick: u64) -> SystemTime {
    const WINDOWS_TICK: u64 = 10000000;
    const SEC_TO_UNIX_EPOCH: u64 = 11644473600;

    let sec = tick / WINDOWS_TICK - SEC_TO_UNIX_EPOCH;
    let nsec = tick % WINDOWS_TICK * 100;
    SystemTime::UNIX_EPOCH + Duration::from_secs(sec) + Duration::from_nanos(nsec)
}

#[derive(Clone)]
pub struct Metadata {
    stat: FileMetadata,
}

impl Metadata {
    /// Create from [`FileMetadata`].
    pub fn from_stat(stat: FileMetadata) -> Self {
        Self { stat }
    }

    pub fn file_type(&self) -> FileType {
        FileType::new(self.stat.dwFileAttributes, self.stat.dwReparseTag)
    }

    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }

    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }

    pub fn is_symlink(&self) -> bool {
        self.file_type().is_symlink()
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.stat.nFileSize
    }

    pub fn permissions(&self) -> Permissions {
        Permissions::new(self.stat.dwFileAttributes)
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(filetime_to_systemtime(self.stat.ftLastWriteTime))
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(filetime_to_systemtime(self.stat.ftLastAccessTime))
    }

    pub fn created(&self) -> io::Result<SystemTime> {
        Ok(filetime_to_systemtime(self.stat.ftCreationTime))
    }

    pub fn file_attributes(&self) -> u32 {
        self.stat.dwFileAttributes
    }

    pub fn creation_time(&self) -> u64 {
        self.stat.ftCreationTime
    }

    pub fn last_access_time(&self) -> u64 {
        self.stat.ftLastAccessTime
    }

    pub fn last_write_time(&self) -> u64 {
        self.stat.ftLastWriteTime
    }

    pub fn volume_serial_number(&self) -> Option<u32> {
        self.stat.dwVolumeSerialNumber
    }

    pub fn number_of_links(&self) -> Option<u32> {
        self.stat.nNumberOfLinks
    }

    pub fn file_index(&self) -> Option<u64> {
        self.stat.nFileIndex
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType {
    attributes: u32,
    reparse_tag: u32,
}

impl FileType {
    fn new(attributes: u32, reparse_tag: u32) -> Self {
        Self {
            attributes,
            reparse_tag,
        }
    }

    pub fn is_dir(&self) -> bool {
        !self.is_symlink() && self.is_directory()
    }

    pub fn is_file(&self) -> bool {
        !self.is_symlink() && !self.is_directory()
    }

    pub fn is_symlink(&self) -> bool {
        self.is_reparse_point() && self.is_reparse_tag_name_surrogate()
    }

    pub fn is_symlink_dir(&self) -> bool {
        self.is_symlink() && self.is_directory()
    }

    pub fn is_symlink_file(&self) -> bool {
        self.is_symlink() && !self.is_directory()
    }

    fn is_directory(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    fn is_reparse_point(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    fn is_reparse_tag_name_surrogate(&self) -> bool {
        self.reparse_tag & 0x20000000 != 0
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions {
    pub(crate) attrs: u32,
}

impl Permissions {
    fn new(attrs: u32) -> Self {
        Self { attrs }
    }

    pub fn readonly(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_READONLY != 0
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            self.attrs |= FILE_ATTRIBUTE_READONLY;
        } else {
            self.attrs &= !FILE_ATTRIBUTE_READONLY;
        }
    }
}
