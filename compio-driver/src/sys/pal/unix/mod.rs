#![allow(unused_imports)]

pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use cfg_if::cfg_if;
use smallvec::SmallVec;

cfg_if! {
    if #[cfg(gnulinux)] {
        pub(crate) use libc::statx as Statx;
        pub use libc::stat64 as Stat;
        pub use libc::openat64 as openat;
    } else {
        pub use libc::stat as Stat;
        pub use libc::openat;
    }
}

cfg_if! {
    if #[cfg(any(
        all(target_os = "linux", not(target_env = "musl")),
        target_os = "android",
        target_os = "l4re",
        target_os = "hurd"
    ))] {
        pub use libc::{ftruncate64, off64_t};
    } else {
        pub use libc::{ftruncate as ftruncate64, off_t as off64_t};
    }
}

cfg_if! {
    if #[cfg(aio)] {
        pub use libc::aiocb as Aiocb;

        pub fn new_aiocb() -> Aiocb {
            unsafe { std::mem::zeroed() }
        }
    } else {
        #[allow(non_camel_case_types)]
        pub type Aiocb = ();

        pub fn new_aiocb() -> Aiocb {}
    }
}

cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android", target_os = "hurd"))] {
        pub use libc::{pread64 as pread, preadv64 as preadv, pwrite64 as pwrite, pwritev64 as pwritev};
    } else {
        pub use libc::{pread, preadv, pwrite, pwritev};
    }
}

cfg_if! {
    if #[cfg(polling)] {
        use crate::WaitArg;

        #[derive(Debug)]
        pub(in crate::sys) struct Track {
            pub arg: WaitArg,
            pub ready: bool,
        }

        impl From<WaitArg> for Track {
            fn from(arg: WaitArg) -> Self {
                Self { arg, ready: false }
            }
        }
    }
}

cfg_if! {
    if #[cfg(io_uring)] {
        pub(crate) fn is_op_supported(code: u8) -> bool {
            #[cfg(feature = "once_cell_try")]
            use std::sync::OnceLock;

            #[cfg(not(feature = "once_cell_try"))]
            use once_cell::sync::OnceCell as OnceLock;

            static PROBE: OnceLock<io_uring::Probe> = OnceLock::new();

            PROBE
                .get_or_try_init(|| {
                    let mut probe = io_uring::Probe::new();

                    io_uring::IoUring::new(2)?
                        .submitter()
                        .register_probe(&mut probe)?;

                    std::io::Result::Ok(probe)
                })
                .map(|probe| probe.is_supported(code))
                .unwrap_or_default()
        }
    }
}

cfg_if! {
    if #[cfg(stub)] {
        pub(crate) fn stub_error() -> std::io::Error {
            std::io::Error::other("Stub driver does not support any operations")
        }

        pub(crate) fn stub_unimpl() -> ! {
            unimplemented!("Stub driver does not support any operations")
        }
    }
}

pub use libc::cmsghdr as CmsgHeader;
/// One item in local or more items on heap.
pub type Multi<T> = SmallVec<[T; 1]>;

/// The interest to poll a file descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interest {
    /// Represents a read operation.
    Readable,
    /// Represents a write operation.
    Writable,
}

/// A special file descriptor that always refers to the current working
/// directory. It represents [`AT_FDCWD`](libc::AT_FDCWD) in libc.
pub struct CurrentDir;

impl AsRawFd for CurrentDir {
    fn as_raw_fd(&self) -> RawFd {
        libc::AT_FDCWD
    }
}

impl AsFd for CurrentDir {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(libc::AT_FDCWD) }
    }
}

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
#[repr(C)]
pub(crate) struct StatxTimestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
    pub __statx_timestamp_pad1: [i32; 1],
}

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
#[repr(C)]
pub(crate) struct Statx {
    pub stx_mask: u32,
    pub stx_blksize: u32,
    pub stx_attributes: u64,
    pub stx_nlink: u32,
    pub stx_uid: u32,
    pub stx_gid: u32,
    pub stx_mode: u16,
    __statx_pad1: [u16; 1],
    pub stx_ino: u64,
    pub stx_size: u64,
    pub stx_blocks: u64,
    pub stx_attributes_mask: u64,
    pub stx_atime: StatxTimestamp,
    pub stx_btime: StatxTimestamp,
    pub stx_ctime: StatxTimestamp,
    pub stx_mtime: StatxTimestamp,
    pub stx_rdev_major: u32,
    pub stx_rdev_minor: u32,
    pub stx_dev_major: u32,
    pub stx_dev_minor: u32,
    pub stx_mnt_id: u64,
    pub stx_dio_mem_align: u32,
    pub stx_dio_offset_align: u32,
    __statx_pad3: [u64; 12],
}

#[cfg(target_os = "linux")]
pub(crate) const fn statx_mask() -> u32 {
    // Set mask to ensure all known fields are filled
    // libc::STATX_ALL | libc::STATX_MNT_ID | libc::STATX_DIOALIGN
    0x3FFF
}

#[cfg(target_os = "linux")]
pub(crate) const fn statx_to_stat(statx: Statx) -> Stat {
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
    stat.st_ctime = statx.stx_btime.tv_sec as _;
    stat.st_ctime_nsec = statx.stx_btime.tv_nsec as _;
    stat
}

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
pub(crate) const fn stat_to_statx(stat: Stat) -> Statx {
    let mut statx: Statx = unsafe { std::mem::zeroed() };
    statx.stx_dev_major = libc::major(stat.st_dev as _) as _;
    statx.stx_dev_minor = libc::minor(stat.st_dev as _) as _;
    statx.stx_ino = stat.st_ino as _;
    statx.stx_nlink = stat.st_nlink as _;
    statx.stx_mode = stat.st_mode as _;
    statx.stx_uid = stat.st_uid as _;
    statx.stx_gid = stat.st_gid as _;
    statx.stx_rdev_major = libc::major(stat.st_rdev as _) as _;
    statx.stx_rdev_minor = libc::minor(stat.st_rdev as _) as _;
    statx.stx_size = stat.st_size as _;
    statx.stx_blksize = stat.st_blksize as _;
    statx.stx_blocks = stat.st_blocks as _;
    statx.stx_atime.tv_sec = stat.st_atime as _;
    statx.stx_atime.tv_nsec = stat.st_atime_nsec as _;
    statx.stx_mtime.tv_sec = stat.st_mtime as _;
    statx.stx_mtime.tv_nsec = stat.st_mtime_nsec as _;
    statx.stx_btime.tv_sec = stat.st_ctime as _;
    statx.stx_btime.tv_nsec = stat.st_ctime_nsec as _;
    statx
}
