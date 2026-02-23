pub use std::fs::Permissions;
use std::{
    io,
    os::{
        fd::AsFd,
        unix::prelude::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    panic::resume_unwind,
    path::Path,
    time::{Duration, SystemTime},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    ToSharedFd,
    op::{CurrentDir, PathStat, Stat},
};

use crate::{File, path_string};

async fn metadata_impl(
    dir: impl AsFd + 'static,
    path: impl AsRef<Path>,
    follow_symlink: bool,
) -> io::Result<Metadata> {
    let path = path_string(path)?;
    let op = PathStat::new(dir, path, follow_symlink);
    let BufResult(res, op) = compio_runtime::submit(op).await;
    res.map(|_| Metadata::from_stat(op.into_inner()))
}

pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(CurrentDir, path, true).await
}

pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(CurrentDir, path, false).await
}

pub async fn metadata_at(dir: &File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(dir.to_shared_fd(), path, true).await
}

pub async fn symlink_metadata_at(dir: &File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_impl(dir.to_shared_fd(), path, false).await
}

pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::set_permissions(path, perm))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

#[derive(Clone)]
pub struct Metadata(pub(crate) Stat);

impl Metadata {
    /// Create from [`Stat`].
    pub fn from_stat(stat: Stat) -> Self {
        Self(stat)
    }

    pub fn file_type(&self) -> FileType {
        FileType(self.0.st_mode as _)
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
        self.0.st_size as _
    }

    pub fn permissions(&self) -> Permissions {
        Permissions::from_mode(self.0.st_mode as _)
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_mtime as _)
            + Duration::from_nanos(self.0.st_mtime_nsec as _))
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_atime as _)
            + Duration::from_nanos(self.0.st_atime_nsec as _))
    }

    #[cfg(not(noctime))]
    pub fn created(&self) -> io::Result<SystemTime> {
        // We've assigned btime field to ctime.
        Ok(SystemTime::UNIX_EPOCH
            + Duration::from_secs(self.0.st_ctime as _)
            + Duration::from_nanos(self.0.st_ctime_nsec as _))
    }

    #[cfg(noctime)]
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
        self.0.st_ino as _
    }

    fn mode(&self) -> u32 {
        self.0.st_mode as _
    }

    fn nlink(&self) -> u64 {
        self.0.st_nlink as _
    }

    fn uid(&self) -> u32 {
        self.0.st_uid as _
    }

    fn gid(&self) -> u32 {
        self.0.st_gid as _
    }

    fn rdev(&self) -> u64 {
        self.0.st_rdev as _
    }

    fn size(&self) -> u64 {
        self.0.st_size as _
    }

    fn atime(&self) -> i64 {
        self.0.st_atime as _
    }

    fn atime_nsec(&self) -> i64 {
        self.0.st_atime_nsec as _
    }

    fn mtime(&self) -> i64 {
        self.0.st_mtime as _
    }

    fn mtime_nsec(&self) -> i64 {
        self.0.st_mtime_nsec as _
    }

    fn ctime(&self) -> i64 {
        self.0.st_ctime as _
    }

    fn ctime_nsec(&self) -> i64 {
        self.0.st_ctime_nsec as _
    }

    fn blksize(&self) -> u64 {
        self.0.st_blksize as _
    }

    fn blocks(&self) -> u64 {
        self.0.st_blocks as _
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType(pub(crate) libc::mode_t);

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.is(libc::S_IFDIR)
    }

    pub fn is_file(&self) -> bool {
        self.is(libc::S_IFREG)
    }

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
