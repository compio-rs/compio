use std::{
    io,
    os::windows::fs::MetadataExt,
    path::Path,
    time::{Duration, SystemTime},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{op::PathStat, syscall};
use compio_runtime::Runtime;
use windows_sys::Win32::{
    Foundation::FILETIME,
    Storage::FileSystem::{
        SetFileAttributesW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
    },
};

use crate::path_string;

async fn metadata_impl(path: impl AsRef<Path>, follow_symlink: bool) -> io::Result<Metadata> {
    let path = path_string(path)?;
    let op = PathStat::new(path, follow_symlink);
    let BufResult(res, op) = Runtime::current().submit(op).await;
    res.map(|_| Metadata::from_path_stat(op.into_inner()))
}

/// Given a path, query the file system to get information about a file,
/// directory, etc.
pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(path, true).await
}

/// Query the metadata about a file without following symlinks.
pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(path, false).await
}

/// Changes the permissions found on a file or a directory.
pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path_string(path)?;
    Runtime::current()
        .spawn_blocking(move || {
            syscall!(BOOL, SetFileAttributesW(path.as_ptr(), perm.attrs))?;
            Ok(())
        })
        .await
}

const fn create_u64(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | (low as u64)
}

const fn filetime_u64(t: FILETIME) -> u64 {
    create_u64(t.dwHighDateTime, t.dwLowDateTime)
}

#[inline]
fn filetime_to_systemtime(t: FILETIME) -> SystemTime {
    const WINDOWS_TICK: u64 = 10000000;
    const SEC_TO_UNIX_EPOCH: u64 = 11644473600;

    let tick = filetime_u64(t);
    let sec = tick / WINDOWS_TICK - SEC_TO_UNIX_EPOCH;
    let nsec = tick % WINDOWS_TICK * 100;
    SystemTime::UNIX_EPOCH + Duration::from_secs(sec) + Duration::from_nanos(nsec)
}

/// Metadata information about a file.
#[derive(Clone)]
pub struct Metadata {
    stat: BY_HANDLE_FILE_INFORMATION,
    reparse_tag: u32,
    handle_info: bool,
}

impl Metadata {
    pub(crate) fn from_stat((stat, reparse_tag): (BY_HANDLE_FILE_INFORMATION, u32)) -> Self {
        Self {
            stat,
            reparse_tag,
            handle_info: true,
        }
    }

    pub(crate) fn from_path_stat(
        (stat, reparse_tag, handle_info): (BY_HANDLE_FILE_INFORMATION, u32, bool),
    ) -> Self {
        Self {
            stat,
            reparse_tag,
            handle_info,
        }
    }

    /// Returns the file type for this metadata.
    pub fn file_type(&self) -> FileType {
        FileType::new(self.stat.dwFileAttributes, self.reparse_tag)
    }

    /// Returns `true` if this metadata is for a directory.
    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }

    /// Returns `true` if this metadata is for a regular file.
    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }

    /// Returns `true` if this metadata is for a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.file_type().is_symlink()
    }

    /// Returns the size of the file, in bytes, this metadata is for.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        create_u64(self.stat.nFileSizeHigh, self.stat.nFileSizeLow)
    }

    /// Returns the permissions of the file this metadata is for.
    pub fn permissions(&self) -> Permissions {
        Permissions::new(self.stat.dwFileAttributes)
    }

    /// Returns the last modification time listed in this metadata.
    ///
    /// The returned value corresponds to the `ftLastWriteTime` field.
    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(filetime_to_systemtime(self.stat.ftLastWriteTime))
    }

    /// Returns the last access time of this metadata.
    ///
    /// The returned value corresponds to the `ftLastAccessTime` field.
    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(filetime_to_systemtime(self.stat.ftLastAccessTime))
    }

    /// Returns the creation time listed in this metadata.
    ///
    /// The returned value corresponds to the `ftCreationTime` field.
    pub fn created(&self) -> io::Result<SystemTime> {
        Ok(filetime_to_systemtime(self.stat.ftCreationTime))
    }
}

impl MetadataExt for Metadata {
    fn file_attributes(&self) -> u32 {
        self.stat.dwFileAttributes
    }

    fn creation_time(&self) -> u64 {
        filetime_u64(self.stat.ftCreationTime)
    }

    fn last_access_time(&self) -> u64 {
        filetime_u64(self.stat.ftLastAccessTime)
    }

    fn last_write_time(&self) -> u64 {
        filetime_u64(self.stat.ftLastWriteTime)
    }

    fn file_size(&self) -> u64 {
        self.len()
    }

    #[cfg(feature = "windows_by_handle")]
    fn volume_serial_number(&self) -> Option<u32> {
        if self.handle_info {
            Some(self.stat.dwVolumeSerialNumber)
        } else {
            None
        }
    }

    #[cfg(feature = "windows_by_handle")]
    fn number_of_links(&self) -> Option<u32> {
        if self.handle_info {
            Some(self.stat.nNumberOfLinks)
        } else {
            None
        }
    }

    #[cfg(feature = "windows_by_handle")]
    fn file_index(&self) -> Option<u64> {
        if self.handle_info {
            Some(create_u64(
                self.stat.nFileIndexHigh,
                self.stat.nFileIndexLow,
            ))
        } else {
            None
        }
    }
}

/// A structure representing a type of file with accessors for each file type.
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

    /// Tests whether this file type represents a directory.
    pub fn is_dir(&self) -> bool {
        !self.is_symlink() && self.is_directory()
    }

    /// Tests whether this file type represents a regular file.
    pub fn is_file(&self) -> bool {
        !self.is_symlink() && !self.is_directory()
    }

    /// Tests whether this file type represents a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.is_reparse_point() && self.is_reparse_tag_name_surrogate()
    }

    /// Returns `true` if this file type is a symbolic link that is also a
    /// directory.
    pub fn is_symlink_dir(&self) -> bool {
        self.is_symlink() && self.is_directory()
    }

    /// Returns `true` if this file type is a symbolic link that is also a file.
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

/// Representation of the various permissions on a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions {
    pub(crate) attrs: u32,
}

impl Permissions {
    fn new(attrs: u32) -> Self {
        Self { attrs }
    }

    /// Returns `true` if these permissions describe a readonly (unwritable)
    /// file.
    pub fn readonly(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_READONLY != 0
    }

    /// Modifies the readonly flag for this set of permissions.
    ///
    /// This operation does **not** modify the files attributes.
    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            self.attrs |= FILE_ATTRIBUTE_READONLY;
        } else {
            self.attrs &= !FILE_ATTRIBUTE_READONLY;
        }
    }
}
