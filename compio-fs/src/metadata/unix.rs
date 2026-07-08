pub use std::fs::Permissions;
use std::{
    hash::Hash,
    io,
    os::{
        fd::AsFd,
        unix::prelude::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::Path,
    time::{Duration, SystemTime},
};

use compio_buf::{BufResult, IntoInner};
#[cfg(dirfd)]
use compio_driver::ToSharedFd;
use compio_driver::op::{CurrentDir, FileAttr, PathStat, Stat};
use compio_runtime::ResumeUnwind;
use rustix::fs::FileType as FileTypeInner;

use crate::path_string;

async fn metadata_impl(
    dir: impl AsFd + 'static,
    path: impl AsRef<Path>,
    follow_symlink: bool,
) -> io::Result<Metadata> {
    let path = path_string(path)?;
    let op = PathStat::new(dir, path, follow_symlink);
    let BufResult(res, op) = compio_runtime::submit(op).await;
    res.map(|_| Metadata::from_attr(op.into_inner()))
}

pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(CurrentDir, path, true).await
}

pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(CurrentDir, path, false).await
}

#[cfg(dirfd)]
pub async fn metadata_at(dir: &crate::File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(dir.to_shared_fd(), path, true).await
}

#[cfg(dirfd)]
pub async fn symlink_metadata_at(
    dir: &crate::File,
    path: impl AsRef<Path>,
) -> io::Result<Metadata> {
    metadata_impl(dir.to_shared_fd(), path, false).await
}

pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::set_permissions(path, perm))
        .await
        .resume_unwind()
        .expect("shouldn't be cancelled")
}

#[derive(Clone)]
pub struct Metadata(pub(crate) FileAttr);

impl Metadata {
    /// Create from [`Stat`].
    ///
    /// The birth (creation) time is only recovered from platforms that store it
    /// in `struct stat` (BSD/Apple); on Linux it is unavailable through a plain
    /// [`Stat`] and [`created`](Self::created) will return an error.
    pub fn from_stat(stat: Stat) -> Self {
        Self(FileAttr::from_stat(stat))
    }

    /// Create from [`FileAttr`].
    pub(crate) fn from_attr(attr: FileAttr) -> Self {
        Self(attr)
    }

    pub fn file_type(&self) -> FileType {
        FileType(FileTypeInner::from_raw_mode(self.0.stat.st_mode as _))
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
        self.0.stat.st_size as _
    }

    pub fn permissions(&self) -> Permissions {
        Permissions::from_mode(self.0.stat.st_mode as _)
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.stat.st_mtime as _)
            + Duration::from_nanos(self.0.stat.st_mtime_nsec as _))
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.stat.st_atime as _)
            + Duration::from_nanos(self.0.stat.st_atime_nsec as _))
    }

    pub fn created(&self) -> io::Result<SystemTime> {
        match self.0.created {
            Some((secs, nsecs)) => Ok(SystemTime::UNIX_EPOCH
                + Duration::from_secs(secs as _)
                + Duration::from_nanos(nsecs as _)),
            None => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "creation time is not available for the filesystem",
            )),
        }
    }
}

impl MetadataExt for Metadata {
    fn dev(&self) -> u64 {
        self.0.stat.st_dev as _
    }

    fn ino(&self) -> u64 {
        self.0.stat.st_ino as _
    }

    fn mode(&self) -> u32 {
        self.0.stat.st_mode as _
    }

    fn nlink(&self) -> u64 {
        self.0.stat.st_nlink as _
    }

    fn uid(&self) -> u32 {
        self.0.stat.st_uid as _
    }

    fn gid(&self) -> u32 {
        self.0.stat.st_gid as _
    }

    fn rdev(&self) -> u64 {
        self.0.stat.st_rdev as _
    }

    fn size(&self) -> u64 {
        self.0.stat.st_size as _
    }

    fn atime(&self) -> i64 {
        self.0.stat.st_atime as _
    }

    fn atime_nsec(&self) -> i64 {
        self.0.stat.st_atime_nsec as _
    }

    fn mtime(&self) -> i64 {
        self.0.stat.st_mtime as _
    }

    fn mtime_nsec(&self) -> i64 {
        self.0.stat.st_mtime_nsec as _
    }

    fn ctime(&self) -> i64 {
        self.0.stat.st_ctime as _
    }

    fn ctime_nsec(&self) -> i64 {
        self.0.stat.st_ctime_nsec as _
    }

    fn blksize(&self) -> u64 {
        self.0.stat.st_blksize as _
    }

    fn blocks(&self) -> u64 {
        self.0.stat.st_blocks as _
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FileType(pub(crate) FileTypeInner);

impl Hash for FileType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_raw_mode().hash(state);
    }
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    pub fn is_file(&self) -> bool {
        self.0.is_file()
    }

    pub fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }
}

impl FileTypeExt for FileType {
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
