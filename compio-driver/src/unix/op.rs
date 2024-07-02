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
pub struct ReadVectoredAt<T: IoVectoredBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> ReadVectoredAt<T, S> {
    /// Create [`ReadVectoredAt`].
    pub fn new(fd: SharedFd<S>, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for ReadVectoredAt<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Write a file at specified position from vectored buffer.
pub struct WriteVectoredAt<T: IoVectoredBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) offset: u64,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> WriteVectoredAt<T, S> {
    /// Create [`WriteVectoredAt`]
    pub fn new(fd: SharedFd<S>, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for WriteVectoredAt<T, S> {
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

impl<S> ShutdownSocket<S> {
    pub(crate) fn how(&self) -> i32 {
        match self.how {
            Shutdown::Write => libc::SHUT_WR,
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Both => libc::SHUT_RDWR,
        }
    }
}

/// Accept a connection.
pub struct Accept<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<S> Accept<S> {
    /// Create [`Accept`].
    pub fn new(fd: SharedFd<S>) -> Self {
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
pub struct Recv<T: IoBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> Recv<T, S> {
    /// Create [`Recv`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut, S> IntoInner for Recv<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    /// Create [`RecvVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> Send<T, S> {
    /// Create [`Send`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf, S> IntoInner for Send<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    /// Create [`SendVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pub(crate) struct RecvFromHeader<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<S> RecvFromHeader<S> {
    pub fn new(fd: SharedFd<S>) -> Self {
        Self {
            fd,
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }

    pub fn into_addr(self) -> (sockaddr_storage, socklen_t) {
        (self.addr, self.msg.msg_namelen)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) header: RecvFromHeader<S>,
    pub(crate) buffer: T,
    pub(crate) slices: [IoSliceMut; 1],
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            header: RecvFromHeader::new(fd),
            buffer,
            // SAFETY: We never use this slice.
            slices: [unsafe { IoSliceMut::from_slice(&mut []) }],
        }
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) header: RecvFromHeader<S>,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T) -> Self {
        Self {
            header: RecvFromHeader::new(fd),
            buffer,
            slices: vec![],
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

pub(crate) struct SendToHeader<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) addr: SockAddr,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<S> SendToHeader<S> {
    pub fn new(fd: SharedFd<S>, addr: SockAddr) -> Self {
        Self {
            fd,
            addr,
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) header: SendToHeader<S>,
    pub(crate) buffer: T,
    pub(crate) slices: [IoSlice; 1],
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: SharedFd<S>, buffer: T, addr: SockAddr) -> Self {
        Self {
            header: SendToHeader::new(fd, addr),
            buffer,
            // SAFETY: We never use this slice.
            slices: [unsafe { IoSlice::from_slice(&[]) }],
        }
    }
}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    pub(crate) header: SendToHeader<S>,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSlice>,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: SharedFd<S>, buffer: T, addr: SockAddr) -> Self {
        Self {
            header: SendToHeader::new(fd, addr),
            buffer,
            slices: vec![],
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// The interest to poll a file descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interest {
    /// Represents a read operation.
    Readable,
    /// Represents a write operation.
    Writable,
}

/// Poll a file descriptor for specified [`Interest`].
pub struct PollOnce<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) interest: Interest,
}

impl<S> PollOnce<S> {
    /// Create [`PollOnce`].
    pub fn new(fd: SharedFd<S>, interest: Interest) -> Self {
        Self { fd, interest }
    }
}
