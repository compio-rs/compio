use std::{ffi::CString, marker::PhantomPinned, mem::MaybeUninit, net::Shutdown};

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

pub(crate) struct RecvMsgHeader<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<S> RecvMsgHeader<S> {
    pub fn new(fd: SharedFd<S>) -> Self {
        Self {
            fd,
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }
}

impl<S> RecvMsgHeader<S> {
    pub fn set_msg(&mut self, slices: &mut [IoSliceMut], control: &mut [MaybeUninit<u8>]) {
        self.msg.msg_name = std::ptr::addr_of_mut!(self.addr) as _;
        self.msg.msg_namelen = std::mem::size_of_val(&self.addr) as _;
        self.msg.msg_iov = slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
        self.msg.msg_control = control.as_mut_ptr() as _;
        self.msg.msg_controllen = control.len() as _;
    }

    pub fn into_addr(self) -> (sockaddr_storage, socklen_t) {
        (self.addr, self.msg.msg_namelen)
    }
}

/// Receive data and source address with ancillary data.
pub struct RecvMsg<T: IoBufMut, C: IoBufMut, S> {
    pub(crate) header: RecvMsgHeader<S>,
    pub(crate) buffer: MsgBuf<T, C>,
    pub(crate) slices: [IoSliceMut; 1],
}

impl<T: IoBufMut, C: IoBufMut, S> RecvMsg<T, C, S> {
    /// Create [`RecvMsg`].
    pub fn new(fd: SharedFd<S>, buffer: MsgBuf<T, C>) -> Self {
        Self {
            header: RecvMsgHeader::new(fd),
            buffer,
            // SAFETY: We never use this slice.
            slices: [unsafe { IoSliceMut::from_slice(&mut []) }],
        }
    }

    pub(crate) fn set_msg(&mut self) {
        self.slices[0] = unsafe { self.buffer.inner.as_io_slice_mut() };
        self.header
            .set_msg(&mut self.slices, self.buffer.control.as_mut_slice());
    }
}

impl<T: IoBufMut, C: IoBufMut, S> IntoInner for RecvMsg<T, C, S> {
    type Inner = (MsgBuf<T, C>, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

/// Receive data and source address with ancillary data into vectored buffer.
pub struct RecvMsgVectored<T: IoVectoredBufMut, C: IoBufMut, S> {
    pub(crate) header: RecvMsgHeader<S>,
    pub(crate) buffer: MsgBuf<T, C>,
    pub(crate) slices: Vec<IoSliceMut>,
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> RecvMsgVectored<T, C, S> {
    /// Create [`RecvMsgVectored`].
    pub fn new(fd: SharedFd<S>, buffer: MsgBuf<T, C>) -> Self {
        Self {
            header: RecvMsgHeader::new(fd),
            buffer,
            slices: vec![],
        }
    }

    pub(crate) fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.inner.as_io_slices_mut() };
        self.header
            .set_msg(&mut self.slices, self.buffer.control.as_mut_slice());
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> IntoInner for RecvMsgVectored<T, C, S> {
    type Inner = (MsgBuf<T, C>, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

pub(crate) struct SendMsgHeader<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) addr: SockAddr,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<S> SendMsgHeader<S> {
    pub fn new(fd: SharedFd<S>, addr: SockAddr) -> Self {
        Self {
            fd,
            addr,
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }
}

impl<S> SendMsgHeader<S> {
    pub fn set_msg(&mut self, slices: &[IoSlice], control: &[u8]) {
        self.msg.msg_name = std::ptr::addr_of_mut!(self.addr) as _;
        self.msg.msg_namelen = std::mem::size_of_val(&self.addr) as _;
        self.msg.msg_iov = slices.as_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
        self.msg.msg_control = control.as_ptr() as _;
        self.msg.msg_controllen = control.len() as _;
    }
}

/// Send data to specified address accompanied by ancillary data.
pub struct SendMsg<T: IoBuf, C: IoBuf, S> {
    pub(crate) header: SendMsgHeader<S>,
    pub(crate) buffer: MsgBuf<T, C>,
    pub(crate) slices: [IoSlice; 1],
}

impl<T: IoBuf, C: IoBuf, S> SendMsg<T, C, S> {
    /// Create [`SendMsg`].
    pub fn new(fd: SharedFd<S>, buffer: MsgBuf<T, C>, addr: SockAddr) -> Self {
        Self {
            header: SendMsgHeader::new(fd, addr),
            buffer,
            // SAFETY: We never use this slice.
            slices: [unsafe { IoSlice::from_slice(&[]) }],
        }
    }

    pub(crate) fn set_msg(&mut self) {
        self.slices[0] = unsafe { self.buffer.inner.as_io_slice() };
        self.header
            .set_msg(&self.slices, self.buffer.control.as_slice());
    }
}

impl<T: IoBuf, C: IoBuf, S> IntoInner for SendMsg<T, C, S> {
    type Inner = MsgBuf<T, C>;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to specified address accompanied by ancillary data from vectored
/// buffer.
pub struct SendMsgVectored<T: IoVectoredBuf, C: IoBuf, S> {
    pub(crate) header: SendMsgHeader<S>,
    pub(crate) buffer: MsgBuf<T, C>,
    pub(crate) slices: Vec<IoSlice>,
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsgVectored<T, C, S> {
    /// Create [`SendMsgVectored`].
    pub fn new(fd: SharedFd<S>, buffer: MsgBuf<T, C>, addr: SockAddr) -> Self {
        Self {
            header: SendMsgHeader::new(fd, addr),
            buffer,
            slices: vec![],
        }
    }

    pub(crate) fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.inner.as_io_slices() };
        self.header
            .set_msg(&self.slices, self.buffer.control.as_slice());
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsgVectored<T, C, S> {
    type Inner = MsgBuf<T, C>;

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
