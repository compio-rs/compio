use std::{
    ffi::CString,
    marker::PhantomPinned,
    net::Shutdown,
    os::fd::{AsFd, AsRawFd, OwnedFd},
    pin::Pin,
};

use compio_buf::{IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use pin_project_lite::pin_project;
use socket2::{SockAddr, SockAddrStorage, socklen_t};

use crate::{op::*, sys::aio::*, sys_slice::*};

#[cfg(not(any(
    all(target_os = "linux", not(target_env = "musl")),
    target_os = "android",
    target_os = "l4re",
    target_os = "hurd"
)))]
use libc::{ftruncate as ftruncate64, off_t as off64_t};
#[cfg(any(
    all(target_os = "linux", not(target_env = "musl")),
    target_os = "android",
    target_os = "l4re",
    target_os = "hurd"
))]
use libc::{ftruncate64, off64_t};

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

#[derive(Debug)]
///  Truncates or extends the underlying file, updating the size of this file to
/// become `size`.
pub struct TruncateFile<S: AsFd> {
    pub(crate) fd: S,
    pub(crate) size: u64,
}

impl<S: AsFd> TruncateFile<S> {
    /// Create [`TruncateFile`].
    pub fn new(fd: S, size: u64) -> Self {
        Self { fd, size }
    }

    pub(crate) fn truncate(&self) -> std::io::Result<usize> {
        let size: off64_t = self
            .size
            .try_into()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        crate::syscall!(ftruncate64(self.fd.as_fd().as_raw_fd(), size)).map(|v| v as _)
    }
}

#[cfg(not(gnulinux))]
pub use libc::stat as Stat;
#[cfg(gnulinux)]
pub use libc::stat64 as Stat;
#[cfg(gnulinux)]
pub(crate) use libc::statx as Statx;

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

pin_project! {
    /// Read a file at specified position into vectored buffer.
    pub struct ReadVectoredAt<T: IoVectoredBufMut, S> {
        pub(crate) fd: S,
        pub(crate) offset: u64,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) aiocb: aiocb,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBufMut, S> ReadVectoredAt<T, S> {
    /// Create [`ReadVectoredAt`].
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
            aiocb: new_aiocb(),
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

pin_project! {
    /// Write a file at specified position from vectored buffer.
    pub struct WriteVectoredAt<T: IoVectoredBuf, S> {
        pub(crate) fd: S,
        pub(crate) offset: u64,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) aiocb: aiocb,
        _p: PhantomPinned,
    }
}
impl<T: IoVectoredBuf, S> WriteVectoredAt<T, S> {
    /// Create [`WriteVectoredAt`]
    pub fn new(fd: S, offset: u64, buffer: T) -> Self {
        Self {
            fd,
            offset,
            buffer,
            slices: vec![],
            aiocb: new_aiocb(),
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

pin_project! {
    /// Receive a file into vectored buffer.
    pub struct ReadVectored<T: IoVectoredBufMut, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBufMut, S> ReadVectored<T, S> {
    /// Create [`ReadVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for ReadVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pin_project! {
    /// Send to a file from vectored buffer.
    pub struct WriteVectored<T: IoVectoredBuf, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBuf, S> WriteVectored<T, S> {
    /// Create [`WriteVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S> IntoInner for WriteVectored<T, S> {
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

pin_project! {
    /// Accept a connection.
    pub struct Accept<S> {
        pub(crate) fd: S,
        pub(crate) buffer: SockAddrStorage,
        pub(crate) addr_len: socklen_t,
        pub(crate) accepted_fd: Option<OwnedFd>,
        _p: PhantomPinned,
    }
}

impl<S> Accept<S> {
    /// Create [`Accept`].
    pub fn new(fd: S) -> Self {
        let buffer = SockAddrStorage::zeroed();
        let addr_len = buffer.size_of();
        Self {
            fd,
            buffer,
            addr_len,
            accepted_fd: None,
            _p: PhantomPinned,
        }
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(mut self) -> SockAddr {
        std::mem::forget(self.accepted_fd.take());
        unsafe { SockAddr::new(self.buffer, self.addr_len) }
    }
}

pin_project! {
    /// Receive data from remote.
    ///
    /// It is only used for socket operations. If you want to read from a pipe, use
    /// [`Read`].
    pub struct Recv<T: IoBufMut, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoBufMut, S> Recv<T, S> {
    /// Create [`Recv`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            flags,
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

pin_project! {
    /// Send data to remote.
    ///
    /// It is only used for socket operations. If you want to write to a pipe, use
    /// [`Write`].
    pub struct Send<T: IoBuf, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoBuf, S> Send<T, S> {
    /// Create [`Send`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            flags,
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

pin_project! {
    /// Receive data from remote into vectored buffer.
    pub struct RecvVectored<T: IoVectoredBufMut, S> {
        pub(crate) msg: libc::msghdr,
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBufMut, S> RecvVectored<T, S> {
    /// Create [`RecvVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            fd,
            buffer,
            slices: vec![],
            flags,
            _p: PhantomPinned,
        }
    }

    pub(crate) fn set_msg(self: Pin<&mut Self>) {
        let this = self.project();

        *this.slices = this.buffer.sys_slices_mut();
        this.msg.msg_iov = this.slices.as_mut_ptr() as _;
        this.msg.msg_iovlen = this.slices.len() as _;
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pin_project! {
    /// Send data to remote from vectored buffer.
    pub struct SendVectored<T: IoVectoredBuf, S> {
        pub(crate) msg: libc::msghdr,
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBuf, S> SendVectored<T, S> {
    /// Create [`SendVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            msg: unsafe { std::mem::zeroed() },
            fd,
            buffer,
            slices: vec![],
            flags,
            _p: PhantomPinned,
        }
    }

    pub(crate) fn set_msg(self: Pin<&mut Self>) {
        let this = self.project();

        *this.slices = this.buffer.as_ref().sys_slices();
        this.msg.msg_iov = this.slices.as_mut_ptr() as _;
        this.msg.msg_iovlen = this.slices.len() as _;
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pin_project! {
    /// Receive data and source address with ancillary data into vectored buffer.
    pub struct RecvMsg<T: IoVectoredBufMut, C: IoBufMut, S> {
        pub(crate) msg: libc::msghdr,
        pub(crate) addr: SockAddrStorage,
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) control: C,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> RecvMsg<T, C, S> {
    /// Create [`RecvMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C, flags: i32) -> Self {
        assert!(
            control.buf_ptr().cast::<libc::cmsghdr>().is_aligned(),
            "misaligned control message buffer"
        );
        Self {
            msg: unsafe { std::mem::zeroed() },
            addr: SockAddrStorage::zeroed(),
            fd,
            buffer,
            control,
            slices: vec![],
            flags,
            _p: PhantomPinned,
        }
    }

    pub(crate) fn set_msg(self: Pin<&mut Self>) {
        let this = self.project();
        *this.slices = this.buffer.sys_slices_mut();

        this.msg.msg_name = this.addr as *mut _ as _;
        this.msg.msg_namelen = this.addr.size_of() as _;
        this.msg.msg_iov = this.slices.as_mut_ptr() as _;
        this.msg.msg_iovlen = this.slices.len() as _;
        this.msg.msg_control = this.control.buf_mut_ptr() as _;
        this.msg.msg_controllen = this.control.buf_capacity() as _;
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S> IntoInner for RecvMsg<T, C, S> {
    type Inner = ((T, C), SockAddrStorage, socklen_t, usize);

    fn into_inner(self) -> Self::Inner {
        (
            (self.buffer, self.control),
            self.addr,
            self.msg.msg_namelen,
            self.msg.msg_controllen as _,
        )
    }
}

pin_project! {
    /// Send data to specified address accompanied by ancillary data from vectored
    /// buffer.
    pub struct SendMsg<T: IoVectoredBuf, C: IoBuf, S> {
        pub(crate) msg: libc::msghdr,
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        #[pin]
        pub(crate) control: C,
        pub(crate) addr: SockAddr,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsg<T, C, S> {
    /// Create [`SendMsg`].
    ///
    /// # Panics
    ///
    /// This function will panic if the control message buffer is misaligned.
    pub fn new(fd: S, buffer: T, control: C, addr: SockAddr, flags: i32) -> Self {
        assert!(
            control.buf_ptr().cast::<libc::cmsghdr>().is_aligned(),
            "misaligned control message buffer"
        );
        Self {
            msg: unsafe { std::mem::zeroed() },
            fd,
            buffer,
            control,
            addr,
            slices: vec![],
            flags,
            _p: PhantomPinned,
        }
    }

    pub(crate) fn set_msg(self: Pin<&mut Self>) {
        let this = self.project();
        *this.slices = this.buffer.as_ref().sys_slices();
        this.msg.msg_name = this.addr.as_ptr() as _;
        this.msg.msg_namelen = this.addr.len();
        this.msg.msg_iov = this.slices.as_ptr() as _;
        this.msg.msg_iovlen = this.slices.len() as _;
        this.msg.msg_control = this.control.buf_ptr() as _;
        this.msg.msg_controllen = this.control.buf_len() as _;
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsg<T, C, S> {
    type Inner = (T, C);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.control)
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
    pub(crate) fd: S,
    pub(crate) interest: Interest,
}

impl<S> PollOnce<S> {
    /// Create [`PollOnce`].
    pub fn new(fd: S, interest: Interest) -> Self {
        Self { fd, interest }
    }
}
