use std::{
    ffi::CString,
    io,
    os::unix::prelude::{FileTypeExt, MetadataExt, OsStrExt, PermissionsExt},
    path::Path,
    time::{Duration, SystemTime},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::op::PathStat;
use compio_runtime::Runtime;

async fn metadata_impl(path: impl AsRef<Path>, follow_symlink: bool) -> io::Result<Metadata> {
    let path = CString::new(path.as_ref().as_os_str().as_bytes().to_vec()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "file name contained an unexpected NUL byte",
        )
    })?;
    let op = PathStat::new(path, follow_symlink);
    let BufResult(res, op) = Runtime::current().submit(op).await;
    res.map(|_| Metadata::from_stat(op.into_inner()))
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

/// Metadata information about a file.
#[derive(Clone)]
pub struct Metadata(pub(crate) libc::stat);

impl Metadata {
    /// Create from [`libc::stat`].
    pub fn from_stat(stat: libc::stat) -> Self {
        Self(stat)
    }

    /// Returns the file type for this metadata.
    pub fn file_type(&self) -> FileType {
        FileType(self.0.st_mode)
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
        self.0.st_size as _
    }

    /// Returns the permissions of the file this metadata is for.
    pub fn permissions(&self) -> Permissions {
        Permissions(self.0.st_mode)
    }

    /// Returns the last modification time listed in this metadata.
    ///
    /// The returned value corresponds to the `mtime` field.
    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_mtime as _)
            + Duration::from_nanos(self.0.st_mtime_nsec as _))
    }

    /// Returns the last access time of this metadata.
    ///
    /// The returned value corresponds to the `atime` field.
    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_atime as _)
            + Duration::from_nanos(self.0.st_atime_nsec as _))
    }

    /// Returns the creation time listed in this metadata.
    ///
    /// The returned value corresponds to the `btime` field of [`libc::statx`].
    #[cfg(not(any(
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
    )))]
    pub fn created(&self) -> io::Result<SystemTime> {
        // We've assigned btime field to ctime.
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_ctime as _)
            + Duration::from_nanos(self.0.st_ctime_nsec as _))
    }

    /// Returns the creation time listed in this metadata.
    ///
    /// The returned value corresponds to the `birthtime` field.
    #[cfg(any(
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
    ))]
    pub fn created(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_birthtime as _)
            + Duration::from_nanos(self.0.st_birthtime_nsec as _))
    }
}

impl MetadataExt for Metadata {
    fn dev(&self) -> u64 {
        self.0.st_dev as _
    }

    fn ino(&self) -> u64 {
        self.0.st_ino
    }

    fn mode(&self) -> u32 {
        self.0.st_mode as _
    }

    fn nlink(&self) -> u64 {
        self.0.st_nlink as _
    }

    fn uid(&self) -> u32 {
        self.0.st_uid
    }

    fn gid(&self) -> u32 {
        self.0.st_gid
    }

    fn rdev(&self) -> u64 {
        self.0.st_rdev as _
    }

    fn size(&self) -> u64 {
        self.0.st_size as _
    }

    fn atime(&self) -> i64 {
        self.0.st_atime
    }

    fn atime_nsec(&self) -> i64 {
        self.0.st_atime_nsec
    }

    fn mtime(&self) -> i64 {
        self.0.st_mtime
    }

    fn mtime_nsec(&self) -> i64 {
        self.0.st_mtime_nsec
    }

    fn ctime(&self) -> i64 {
        self.0.st_ctime
    }

    fn ctime_nsec(&self) -> i64 {
        self.0.st_ctime_nsec
    }

    fn blksize(&self) -> u64 {
        self.0.st_blksize as _
    }

    fn blocks(&self) -> u64 {
        self.0.st_blocks as _
    }
}

/// A structure representing a type of file with accessors for each file type.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType(pub(crate) libc::mode_t);

impl FileType {
    /// Tests whether this file type represents a directory.
    pub fn is_dir(&self) -> bool {
        self.is(libc::S_IFDIR)
    }

    /// Tests whether this file type represents a regular file.
    pub fn is_file(&self) -> bool {
        self.is(libc::S_IFREG)
    }

    /// Tests whether this file type represents a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.is(libc::S_IFLNK)
    }

    fn is(&self, mode: libc::mode_t) -> bool {
        self.masked() == mode
    }

    fn masked(&self) -> libc::mode_t {
        self.0 & libc::S_IFMT
    }
}

impl FileTypeExt for FileType {
    fn is_block_device(&self) -> bool {
        self.is(libc::S_IFBLK)
    }

    fn is_char_device(&self) -> bool {
        self.is(libc::S_IFCHR)
    }

    fn is_fifo(&self) -> bool {
        self.is(libc::S_IFIFO)
    }

    fn is_socket(&self) -> bool {
        self.is(libc::S_IFSOCK)
    }
}

/// Representation of the various permissions on a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Permissions(pub(crate) libc::mode_t);

impl Permissions {
    /// Returns `true` if these permissions describe a readonly (unwritable)
    /// file.
    pub fn readonly(&self) -> bool {
        self.0 & 0o222 == 0
    }

    /// Modifies the readonly flag for this set of permissions.
    ///
    /// This operation does **not** modify the files attributes.
    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            // remove write permission for all classes; equivalent to `chmod a-w <file>`
            self.0 &= !0o222;
        } else {
            // add write permission for all classes; equivalent to `chmod a+w <file>`
            self.0 |= 0o222;
        }
    }
}

impl PermissionsExt for Permissions {
    fn mode(&self) -> u32 {
        self.0 as _
    }

    fn set_mode(&mut self, mode: u32) {
        self.0 = mode as _;
    }

    fn from_mode(mode: u32) -> Self {
        Self(mode as _)
    }
}
