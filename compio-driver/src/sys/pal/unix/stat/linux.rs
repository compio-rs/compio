use rustix::fs::{self, Statx, StatxFlags};

use super::*;

pub const STATX_MASK: StatxFlags = StatxFlags::ALL
    .union(StatxFlags::MNT_ID)
    .union(StatxFlags::DIOALIGN);

pub fn statx<Fd: AsFd>(dirfd: Fd, path: &CStr, follow_symlink: bool) -> io::Result<Statx> {
    let mut flags = AtFlags::EMPTY_PATH;
    if !follow_symlink {
        flags |= AtFlags::SYMLINK_NOFOLLOW;
    }

    let statx = fs::statx(dirfd, path, flags, STATX_MASK)?;
    Ok(statx)
}

#[allow(dead_code)]
pub fn stat<Fd: AsFd>(dirfd: Fd, path: &CStr, follow_symlink: bool) -> io::Result<FileAttr> {
    statx(dirfd, path, follow_symlink).map(statx_to_attr)
}

/// Convert a [`Statx`] into a [`FileAttr`], carrying the birth time separately
/// from the [`Stat`] (which on Linux has no birth-time field).
pub fn statx_to_attr(statx: Statx) -> FileAttr {
    FileAttr {
        stat: statx_to_stat(statx),
        created: statx_created(&statx),
    }
}

/// Extract the birth (creation) time from a [`Statx`], if the kernel/filesystem
/// reported it.
fn statx_created(statx: &Statx) -> Option<(i64, i64)> {
    if statx.stx_mask & StatxFlags::BTIME.bits() != 0 {
        Some((statx.stx_btime.tv_sec as _, statx.stx_btime.tv_nsec as _))
    } else {
        None
    }
}

pub const fn statx_to_stat(statx: Statx) -> Stat {
    let mut stat: Stat = unsafe { std::mem::zeroed() };
    stat.st_dev = libc::makedev(statx.stx_dev_major, statx.stx_dev_minor) as _;
    stat.st_ino = statx.stx_ino as _;
    stat.st_nlink = statx.stx_nlink as _;
    stat.st_mode = statx.stx_mode as _;
    stat.st_uid = statx.stx_uid as _;
    stat.st_gid = statx.stx_gid as _;
    stat.st_rdev = libc::makedev(statx.stx_rdev_major, statx.stx_rdev_minor) as _;
    stat.st_size = statx.stx_size as _;
    stat.st_blksize = statx.stx_blksize as _;
    stat.st_blocks = statx.stx_blocks as _;
    stat.st_atime = statx.stx_atime.tv_sec as _;
    stat.st_atime_nsec = statx.stx_atime.tv_nsec as _;
    stat.st_mtime = statx.stx_mtime.tv_sec as _;
    stat.st_mtime_nsec = statx.stx_mtime.tv_nsec as _;
    // `st_ctime` is the inode change time, not the birth time. The birth time
    // is carried separately in `FileAttr::created`.
    stat.st_ctime = statx.stx_ctime.tv_sec as _;
    stat.st_ctime_nsec = statx.stx_ctime.tv_nsec as _;
    stat
}
