#[cfg(unix)]
#[path = "unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "windows.rs"]
mod sys;

#[cfg(windows)]
use std::os::windows::fs::{FileTypeExt, MetadataExt};
use std::{io, path::Path, time::SystemTime};

/// Given a path, query the file system to get information about a file,
/// directory, etc.
pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    sys::metadata(path).await.map(Metadata)
}

/// Query the metadata about a file without following symlinks.
pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    sys::symlink_metadata(path).await.map(Metadata)
}

/// Changes the permissions found on a file or a directory.
pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    sys::set_permissions(path, perm.0).await
}

/// Metadata information about a file.
#[derive(Clone)]
pub struct Metadata(sys::Metadata);

impl Metadata {
    /// Returns the file type for this metadata.
    pub fn file_type(&self) -> FileType {
        FileType(self.0.file_type())
    }

    /// Returns `true` if this metadata is for a directory.
    pub fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    /// Returns `true` if this metadata is for a regular file.
    pub fn is_file(&self) -> bool {
        self.0.is_file()
    }

    /// Returns `true` if this metadata is for a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }

    /// Returns the size of the file, in bytes, this metadata is for.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.0.len()
    }

    /// Returns the permissions of the file this metadata is for.
    pub fn permissions(&self) -> Permissions {
        Permissions(self.0.permissions())
    }

    /// Returns the last modification time listed in this metadata.
    ///
    /// ## Platform specific
    /// * Windows: The returned value corresponds to the `ftLastWriteTime`
    ///   field.
    /// * Unix: The returned value corresponds to the `mtime` field.
    pub fn modified(&self) -> io::Result<SystemTime> {
        self.0.modified()
    }

    /// Returns the last access time of this metadata.
    ///
    /// ## Platform specific
    /// * Windows: The returned value corresponds to the `ftLastAccessTime`
    ///   field.
    /// * Unix: The returned value corresponds to the `atime` field.
    pub fn accessed(&self) -> io::Result<SystemTime> {
        self.0.accessed()
    }

    /// Returns the creation time listed in this metadata.
    ///
    /// ## Platform specific
    /// * Windows: The returned value corresponds to the `ftCreationTime` field.
    /// * Unix: The returned value corresponds to `st_ctime` or `st_birthtime`
    ///   of
    #[cfg_attr(unix, doc = "[`libc::stat`](struct@libc::stat).")]
    #[cfg_attr(
        windows,
        doc = "[`libc::stat`](https://docs.rs/libc/latest/libc/struct.stat.html)."
    )]
    pub fn created(&self) -> io::Result<SystemTime> {
        self.0.created()
    }
}

// The below methods are Windows specific. We cannot impl `MetadataExt` because
// it is going to be sealed.
#[cfg(windows)]
impl Metadata {
    /// Create [`Metadata`] from [`std::fs::Metadata`].
    pub fn from_std(m: std::fs::Metadata) -> Self {
        Self(m)
    }

    /// Returns the value of the `dwFileAttributes` field of this metadata.
    pub fn file_attributes(&self) -> u32 {
        self.0.file_attributes()
    }

    /// Returns the value of the `ftCreationTime` field of this metadata.
    pub fn creation_time(&self) -> u64 {
        self.0.creation_time()
    }

    /// Returns the value of the `ftLastAccessTime` field of this metadata.
    pub fn last_access_time(&self) -> u64 {
        self.0.last_access_time()
    }

    /// Returns the value of the `ftLastWriteTime` field of this metadata.
    pub fn last_write_time(&self) -> u64 {
        self.0.last_write_time()
    }
}

#[cfg(all(windows, feature = "windows_by_handle"))]
impl Metadata {
    /// Returns the value of the `dwVolumeSerialNumber` field of this
    /// metadata.
    pub fn volume_serial_number(&self) -> Option<u32> {
        self.0.volume_serial_number()
    }

    /// Returns the value of the `nNumberOfLinks` field of this
    /// metadata.
    pub fn number_of_links(&self) -> Option<u32> {
        self.0.number_of_links()
    }

    /// Returns the value of the `nFileIndex{Low,High}` fields of this
    /// metadata.
    pub fn file_index(&self) -> Option<u64> {
        self.0.file_index()
    }
}

#[cfg(unix)]
impl Metadata {
    /// Create from [`libc::stat`]
    ///
    /// [`libc::stat`]: struct@libc::stat
    pub fn from_stat(stat: libc::stat) -> Self {
        Self(sys::Metadata::from_stat(stat))
    }
}

#[cfg(unix)]
impl std::os::unix::prelude::MetadataExt for Metadata {
    fn dev(&self) -> u64 {
        self.0.dev()
    }

    fn ino(&self) -> u64 {
        self.0.ino()
    }

    fn mode(&self) -> u32 {
        self.0.mode()
    }

    fn nlink(&self) -> u64 {
        self.0.nlink()
    }

    fn uid(&self) -> u32 {
        self.0.uid()
    }

    fn gid(&self) -> u32 {
        self.0.gid()
    }

    fn rdev(&self) -> u64 {
        self.0.rdev()
    }

    fn size(&self) -> u64 {
        self.0.size()
    }

    fn atime(&self) -> i64 {
        self.0.atime()
    }

    fn atime_nsec(&self) -> i64 {
        self.0.atime_nsec()
    }

    fn mtime(&self) -> i64 {
        self.0.mtime()
    }

    fn mtime_nsec(&self) -> i64 {
        self.0.mtime_nsec()
    }

    fn ctime(&self) -> i64 {
        self.0.ctime()
    }

    fn ctime_nsec(&self) -> i64 {
        self.0.ctime_nsec()
    }

    fn blksize(&self) -> u64 {
        self.0.blksize()
    }

    fn blocks(&self) -> u64 {
        self.0.blocks()
    }
}

/// A structure representing a type of file with accessors for each file type.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType(sys::FileType);

impl FileType {
    /// Tests whether this file type represents a directory.
    pub fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    /// Tests whether this file type represents a regular file.
    pub fn is_file(&self) -> bool {
        self.0.is_file()
    }

    /// Tests whether this file type represents a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }
}

// The below methods are Windows specific. We cannot impl `FileTypeExt` because
// it is sealed.

#[cfg(windows)]
impl FileType {
    /// Returns `true` if this file type is a symbolic link that is also a
    /// directory.
    pub fn is_symlink_dir(&self) -> bool {
        self.0.is_symlink_dir()
    }

    /// Returns `true` if this file type is a symbolic link that is also a file.
    pub fn is_symlink_file(&self) -> bool {
        self.0.is_symlink_file()
    }
}

#[cfg(unix)]
impl std::os::unix::prelude::FileTypeExt for FileType {
    fn is_block_device(&self) -> bool {
        self.0.is_block_device()
    }

    fn is_char_device(&self) -> bool {
        self.0.is_char_device()
    }

    fn is_fifo(&self) -> bool {
        self.0.is_fifo()
    }

    fn is_socket(&self) -> bool {
        self.0.is_socket()
    }
}

/// Representation of the various permissions on a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions(pub(crate) sys::Permissions);

impl Permissions {
    /// Returns `true` if these permissions describe a readonly (unwritable)
    /// file.
    pub fn readonly(&self) -> bool {
        self.0.readonly()
    }

    /// Modifies the readonly flag for this set of permissions.
    ///
    /// This operation does **not** modify the files attributes.
    pub fn set_readonly(&mut self, readonly: bool) {
        self.0.set_readonly(readonly)
    }
}

#[cfg(unix)]
impl std::os::unix::prelude::PermissionsExt for Permissions {
    fn mode(&self) -> u32 {
        self.0.mode()
    }

    fn set_mode(&mut self, mode: u32) {
        self.0.set_mode(mode)
    }

    fn from_mode(mode: u32) -> Self {
        Self(sys::Permissions::from_mode(mode))
    }
}
