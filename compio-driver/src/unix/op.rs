use std::{ffi::CString, marker::PhantomPinned, net::Shutdown};

use compio_buf::{
    IntoInner, IoBuf, IoBufMut, IoSlice, IoSliceMut, IoVectoredBuf, IoVectoredBufMut,
};
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

use crate::{op::*, SharedFd};

/// Open or create a file with flags and mode.
pub struct OpenFile {
    pub(crate) path: CString,
    pub(crate) flags: i32,
    pub(crate) mode: libc::mode_t,
}

impl OpenFile {
    /// Create [`OpenFile`].
    pub fn new(path: CString, flags: i32, mode: libc::mode_t) -> Self {
        Self { path, flags, mode }
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) type Statx = libc::statx;

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
pub(crate) const fn statx_to_stat(statx: Statx) -> libc::stat {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    stat.st_dev = libc::makedev(statx.stx_dev_major, statx.stx_dev_minor);
    stat.st_ino = statx.stx_ino;
    stat.st_nlink = statx.stx_nlink as _;
    stat.st_mode = statx.stx_mode as _;
    stat.st_uid = statx.stx_uid;
    stat.st_gid = statx.stx_gid;
    stat.st_rdev = libc::makedev(statx.stx_rdev_major, statx.stx_rdev_minor);
    stat.st_size = statx.stx_size as _;
    stat.st_blksize = statx.stx_blksize as _;
    stat.st_blocks = statx.stx_blocks as _;
    stat.st_atime = statx.stx_atime.tv_sec;
    stat.st_atime_nsec = statx.stx_atime.tv_nsec as _;
    stat.st_mtime = statx.stx_mtime.tv_sec;
    stat.st_mtime_nsec = statx.stx_mtime.tv_nsec as _;
    stat.st_ctime = statx.stx_btime.tv_sec;
    stat.st_ctime_nsec = statx.stx_btime.tv_nsec as _;
    stat
}

/// Read a file at specified position into vectored buffer.
pub struct ReadVectoredAt<T: IoVectoredBufMut> {
    pub(crate) fd: SharedFd,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut> ReadVectoredAt<T> {
    /// Create [`ReadVectoredAt`].
    pub fn new(fd: SharedFd, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for ReadVectoredAt<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from vectored buffer.
pub struct WriteVectoredAt<T: IoVectoredBuf> {
    pub(crate) fd: SharedFd,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf> WriteVectoredAt<T> {
    /// Create [`WriteVectoredAt`]
    pub fn new(fd: SharedFd, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf> IntoInner for WriteVectoredAt<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Remove file or directory.
pub struct Unlink {
    pub(crate) path: CString,
    pub(crate) dir: bool,
}

impl Unlink {
    /// Create [`Unlink`].
    pub fn new(path: CString, dir: bool) -> Self {
        Self { path, dir }
    }
}

/// Create a directory.
pub struct CreateDir {
    pub(crate) path: CString,
    pub(crate) mode: libc::mode_t,
}

impl CreateDir {
    /// Create [`CreateDir`].
    pub fn new(path: CString, mode: libc::mode_t) -> Self {
        Self { path, mode }
    }
}

/// Rename a file or directory.
pub struct Rename {
    pub(crate) old_path: CString,
    pub(crate) new_path: CString,
}

impl Rename {
    /// Create [`Rename`].
    pub fn new(old_path: CString, new_path: CString) -> Self {
        Self { old_path, new_path }
    }
}

/// Create a symlink.
pub struct Symlink {
    pub(crate) source: CString,
    pub(crate) target: CString,
}

impl Symlink {
    /// Create [`Symlink`]. `target` is a symlink to `source`.
    pub fn new(source: CString, target: CString) -> Self {
        Self { source, target }
    }
}

/// Create a hard link.
pub struct HardLink {
    pub(crate) source: CString,
    pub(crate) target: CString,
}

impl HardLink {
    /// Create [`HardLink`]. `target` is a hard link to `source`.
    pub fn new(source: CString, target: CString) -> Self {
        Self { source, target }
    }
}

/// Create a socket.
pub struct CreateSocket {
    pub(crate) domain: i32,
    pub(crate) socket_type: i32,
    pub(crate) protocol: i32,
}

impl CreateSocket {
    /// Create [`CreateSocket`].
    pub fn new(domain: i32, socket_type: i32, protocol: i32) -> Self {
        Self {
            domain,
            socket_type,
            protocol,
        }
    }
}

impl ShutdownSocket {
    pub(crate) fn how(&self) -> i32 {
        match self.how {
            Shutdown::Write => libc::SHUT_WR,
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Both => libc::SHUT_RDWR,
        }
    }
}

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: SharedFd,
    pub(crate) buffer: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl Accept {
    /// Create [`Accept`].
    pub fn new(fd: SharedFd) -> Self {
        Self {
            fd,
            buffer: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
            _p: PhantomPinned,
        }
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> SockAddr {
        unsafe { SockAddr::new(self.buffer, self.addr_len) }
    }
}

/// Receive data from remote.
pub struct Recv<T: IoBufMut> {
    pub(crate) fd: SharedFd,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBufMut> Recv<T> {
    /// Create [`Recv`].
    pub fn new(fd: SharedFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut> IntoInner for Recv<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut> {
    pub(crate) fd: SharedFd,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut> RecvVectored<T> {
    /// Create [`RecvVectored`].
    pub fn new(fd: SharedFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf> {
    pub(crate) fd: SharedFd,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBuf> Send<T> {
    /// Create [`Send`].
    pub fn new(fd: SharedFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf> IntoInner for Send<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf> {
    pub(crate) fd: SharedFd,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf> SendVectored<T> {
    /// Create [`SendVectored`].
    pub fn new(fd: SharedFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf> IntoInner for SendVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
