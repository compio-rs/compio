use super::*;

/// File metadata returned by the stat operations.
///
/// It bundles the raw platform [`Stat`] together with the file creation (birth)
/// time. On Linux the birth time is not part of `struct stat`; it comes from
/// `statx` instead, so it has to be carried separately. On the BSDs and Apple
/// platforms it is extracted from the `st_birthtime` field of [`Stat`].
#[derive(Clone, Copy)]
pub struct FileAttr {
    /// The raw `stat` result.
    pub stat: Stat,
    /// Creation (birth) time as `(seconds, nanoseconds)` since the Unix epoch,
    /// or [`None`] when the platform/filesystem does not report it.
    pub created: Option<(i64, i64)>,
}

impl FileAttr {
    /// Build from a plain [`Stat`], extracting the birth time from the
    /// `st_birthtime` field when the platform stores it there.
    pub fn from_stat(stat: Stat) -> Self {
        Self {
            created: stat_created(&stat),
            stat,
        }
    }
}

#[cfg(noctime)]
fn stat_created(stat: &Stat) -> Option<(i64, i64)> {
    Some((stat.st_birthtime as _, stat.st_birthtime_nsec as _))
}

#[cfg(not(noctime))]
fn stat_created(_stat: &Stat) -> Option<(i64, i64)> {
    // On Linux `struct stat` has no birth time; it is obtained from `statx`.
    None
}

cfg_select! {
     linux_all => {
        mod_use![linux];
    }
    _ => {
        pub fn stat<Fd: AsFd>(dirfd: Fd, path: &CStr, follow_symlink: bool) -> io::Result<FileAttr> {
            use rustix::fs::{fstat, statat};

            let stat = if path.is_empty() {
                fstat(dirfd)?
            } else {
                let flags = if follow_symlink {
                    AtFlags::empty()
                } else {
                    AtFlags::SYMLINK_NOFOLLOW
                };
                statat(dirfd, path, flags)?
            };
            Ok(FileAttr::from_stat(stat))
        }
    }
}
