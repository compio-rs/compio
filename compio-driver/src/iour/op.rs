use std::{ffi::CString, os::fd::RawFd, pin::Pin};

use compio_buf::{
    BufResult, IntoInner, IoBuf, IoBufMut, IoSlice, IoSliceMut, IoVectoredBuf, IoVectoredBufMut,
};
use io_uring::{
    opcode,
    types::{Fd, FsyncFlags},
};
use libc::{sockaddr_storage, socklen_t};
use socket2::SockAddr;

use super::OpCode;
pub use crate::unix::op::*;
use crate::{op::*, OpEntry};

impl<
    D: std::marker::Send + Unpin + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + std::marker::Sync + Unpin + 'static,
> OpCode for Asyncify<F, D>
{
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(mut self: Pin<&mut Self>) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        self.data = Some(data);
        res
    }
}

impl OpCode for OpenFile {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::OpenAt::new(Fd(libc::AT_FDCWD), self.path.as_ptr())
            .flags(self.flags)
            .mode(self.mode)
            .build()
            .into()
    }
}

impl OpCode for CloseFile {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Close::new(Fd(self.fd)).build().into()
    }
}

/// Get metadata of an opened file.
pub struct FileStat {
    pub(crate) fd: RawFd,
    pub(crate) stat: libc::statx,
}

impl FileStat {
    /// Create [`FileStat`].
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            stat: unsafe { std::mem::zeroed() },
        }
    }
}

impl OpCode for FileStat {
    fn create_entry(mut self: Pin<&mut Self>) -> io_uring::squeue::Entry {
        static EMPTY_NAME: &[u8] = b"\0";
        opcode::Statx::new(
            Fd(self.fd),
            EMPTY_NAME.as_ptr().cast(),
            std::ptr::addr_of_mut!(self.stat).cast(),
        )
        .flags(libc::AT_EMPTY_PATH)
        .build()
    }
}

impl IntoInner for FileStat {
    type Inner = libc::stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

/// Get metadata from path.
pub struct PathStat {
    pub(crate) path: CString,
    pub(crate) stat: libc::statx,
    pub(crate) follow_symlink: bool,
}

impl PathStat {
    /// Create [`PathStat`].
    pub fn new(path: CString, follow_symlink: bool) -> Self {
        Self {
            path,
            stat: unsafe { std::mem::zeroed() },
            follow_symlink,
        }
    }
}

impl OpCode for PathStat {
    fn create_entry(mut self: Pin<&mut Self>) -> io_uring::squeue::Entry {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        opcode::Statx::new(
            Fd(libc::AT_FDCWD),
            self.path.as_ptr(),
            std::ptr::addr_of_mut!(self.stat).cast(),
        )
        .flags(flags)
        .build()
    }
}

impl IntoInner for PathStat {
    type Inner = libc::stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        let fd = Fd(self.fd);
        let slice = self.buffer.as_mut_slice();
        opcode::Read::new(fd, slice.as_mut_ptr() as _, slice.len() as _)
            .offset(self.offset)
            .build()
            .into()
    }
}

impl<T: IoVectoredBufMut> OpCode for ReadVectoredAt<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        opcode::Readv::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _)
            .offset(self.offset)
            .build()
            .into()
    }
}

impl<T: IoVectoredBuf> OpCode for WriteVectoredAt<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.slices = unsafe { self.buffer.as_io_slices() };
        opcode::Write::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

impl OpCode for Sync {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Fsync::new(Fd(self.fd))
            .flags(if self.datasync {
                FsyncFlags::DATASYNC
            } else {
                FsyncFlags::empty()
            })
            .build()
            .into()
    }
}

impl OpCode for ShutdownSocket {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Shutdown::new(Fd(self.fd), self.how())
            .build()
            .into()
    }
}

impl OpCode for CloseSocket {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Close::new(Fd(self.fd)).build().into()
    }
}

impl OpCode for Accept {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        opcode::Accept::new(
            Fd(self.fd),
            &mut self.buffer as *mut sockaddr_storage as *mut libc::sockaddr,
            &mut self.addr_len,
        )
        .build()
        .into()
    }
}

impl OpCode for Connect {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Connect::new(Fd(self.fd), self.addr.as_ptr(), self.addr.len())
            .build()
            .into()
    }
}

impl<T: IoBufMut> OpCode for Recv<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        let fd = self.fd;
        let slice = self.buffer.as_mut_slice();
        opcode::Read::new(Fd(fd), slice.as_mut_ptr() as _, slice.len() as _)
            .build()
            .into()
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        opcode::Readv::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
        .into()
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let slice = self.buffer.as_slice();
        opcode::Write::new(Fd(self.fd), slice.as_ptr(), slice.len() as _)
            .build()
            .into()
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.slices = unsafe { self.buffer.as_io_slices() };
        opcode::Writev::new(
            Fd(self.fd),
            self.slices.as_ptr() as _,
            self.slices.len() as _,
        )
        .build()
        .into()
    }
}

struct RecvFromHeader {
    pub(crate) fd: RawFd,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
}

impl RecvFromHeader {
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
        }
    }

    pub fn create_entry(&mut self, slices: &mut [IoSliceMut]) -> OpEntry {
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: std::mem::size_of_val(&self.addr) as _,
            msg_iov: slices.as_mut_ptr() as _,
            msg_iovlen: slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::RecvMsg::new(Fd(self.fd), &mut self.msg)
            .build()
            .into()
    }

    pub fn into_addr(self) -> (sockaddr_storage, socklen_t) {
        (self.addr, self.msg.msg_namelen)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    header: RecvFromHeader,
    buffer: T,
    slice: [IoSliceMut; 1],
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            header: RecvFromHeader::new(fd),
            buffer,
            // SAFETY: We never use this slice.
            slice: [unsafe { IoSliceMut::from_slice(&mut []) }],
        }
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        let this = &mut *self;
        this.slice[0] = unsafe { this.buffer.as_io_slice_mut() };
        this.header.create_entry(&mut this.slice)
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut> {
    header: RecvFromHeader,
    buffer: T,
    slice: Vec<IoSliceMut>,
}

impl<T: IoVectoredBufMut> RecvFromVectored<T> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            header: RecvFromHeader::new(fd),
            buffer,
            slice: vec![],
        }
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        let this = &mut *self;
        this.slice = unsafe { this.buffer.as_io_slices_mut() };
        this.header.create_entry(&mut this.slice)
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvFromVectored<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

struct SendToHeader {
    pub(crate) fd: RawFd,
    pub(crate) addr: SockAddr,
    pub(crate) msg: libc::msghdr,
}

impl SendToHeader {
    pub fn new(fd: RawFd, addr: SockAddr) -> Self {
        Self {
            fd,
            addr,
            msg: unsafe { std::mem::zeroed() },
        }
    }

    pub fn create_entry(&mut self, slices: &mut [IoSlice]) -> OpEntry {
        self.msg = libc::msghdr {
            msg_name: self.addr.as_ptr() as _,
            msg_namelen: self.addr.len(),
            msg_iov: slices.as_mut_ptr() as _,
            msg_iovlen: slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        opcode::SendMsg::new(Fd(self.fd), &self.msg).build().into()
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    header: SendToHeader,
    buffer: T,
    slice: [IoSlice; 1],
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            header: SendToHeader::new(fd, addr),
            buffer,
            // SAFETY: We never use this slice.
            slice: [unsafe { IoSlice::from_slice(&[]) }],
        }
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        let this = &mut *self;
        this.slice[0] = unsafe { this.buffer.as_io_slice() };
        this.header.create_entry(&mut this.slice)
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf> {
    header: SendToHeader,
    buffer: T,
    slice: Vec<IoSlice>,
}

impl<T: IoVectoredBuf> SendToVectored<T> {
    /// Create [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            header: SendToHeader::new(fd, addr),
            buffer,
            slice: vec![],
        }
    }
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        let this = &mut *self;
        this.slice = unsafe { this.buffer.as_io_slices() };
        this.header.create_entry(&mut this.slice)
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
