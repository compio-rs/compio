use std::{
    ffi::CString,
    io,
    marker::PhantomPinned,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    pin::Pin,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use io_uring::{
    opcode,
    types::{Fd, FsyncFlags},
};
use pin_project_lite::pin_project;
use socket2::{SockAddr, SockAddrStorage, socklen_t};

use super::OpCode;
pub use crate::sys::unix_op::*;
use crate::{OpEntry, op::*, sys_slice::*, syscall};

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(self: Pin<&mut Self>) -> std::io::Result<usize> {
        let this = self.project();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        *this.data = Some(data);
        res
    }
}

impl<
    S,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd<S, F, D>
{
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(self: Pin<&mut Self>) -> std::io::Result<usize> {
        let this = self.project();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(this.fd);
        *this.data = Some(data);
        res
    }
}

impl OpCode for OpenFile {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::OpenAt::new(Fd(libc::AT_FDCWD), self.path.as_ptr())
            .flags(self.flags | libc::O_CLOEXEC)
            .mode(self.mode)
            .build()
            .into()
    }
}

impl OpCode for CloseFile {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Close::new(Fd(self.fd.as_fd().as_raw_fd()))
            .build()
            .into()
    }
}

pin_project! {
    /// Get metadata of an opened file.
    pub struct FileStat<S> {
        pub(crate) fd: S,
        pub(crate) stat: Statx,
    }
}

impl<S> FileStat<S> {
    /// Create [`FileStat`].
    pub fn new(fd: S) -> Self {
        Self {
            fd,
            stat: unsafe { std::mem::zeroed() },
        }
    }
}

impl<S: AsFd> OpCode for FileStat<S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        static EMPTY_NAME: &[u8] = b"\0";
        opcode::Statx::new(
            Fd(this.fd.as_fd().as_fd().as_raw_fd()),
            EMPTY_NAME.as_ptr().cast(),
            this.stat as *mut _ as _,
        )
        .flags(libc::AT_EMPTY_PATH)
        .build()
        .into()
    }
}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

/// Get metadata from path.
pub struct PathStat {
    pub(crate) path: CString,
    pub(crate) stat: Statx,
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
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
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
        .into()
    }
}

impl IntoInner for PathStat {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        let fd = Fd(this.fd.as_fd().as_raw_fd());
        let slice = this.buffer.sys_slice_mut();
        opcode::Read::new(fd, slice.ptr() as _, slice.len() as _)
            .offset(*this.offset)
            .build()
            .into()
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        *this.slices = this.buffer.sys_slices_mut();
        opcode::Readv::new(
            Fd(this.fd.as_fd().as_raw_fd()),
            this.slices.as_ptr() as _,
            this.slices.len() as _,
        )
        .offset(*this.offset)
        .build()
        .into()
    }
}

impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Write::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len() as _,
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        *this.slices = this.buffer.as_ref().sys_slices();
        opcode::Writev::new(
            Fd(this.fd.as_fd().as_raw_fd()),
            this.slices.as_ptr() as _,
            this.slices.len() as _,
        )
        .offset(*this.offset)
        .build()
        .into()
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.project().buffer.sys_slice_mut();
        opcode::Read::new(Fd(fd), slice.ptr() as _, slice.len() as _)
            .build()
            .into()
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        *this.slices = this.buffer.sys_slices_mut();
        opcode::Readv::new(
            Fd(this.fd.as_fd().as_raw_fd()),
            this.slices.as_ptr() as _,
            this.slices.len() as _,
        )
        .build()
        .into()
    }
}

impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Write::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len() as _,
        )
        .build()
        .into()
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        *this.slices = this.buffer.as_ref().sys_slices();
        opcode::Writev::new(
            Fd(this.fd.as_fd().as_raw_fd()),
            this.slices.as_ptr() as _,
            this.slices.len() as _,
        )
        .build()
        .into()
    }
}

impl<S: AsFd> OpCode for Sync<S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Fsync::new(Fd(self.fd.as_fd().as_raw_fd()))
            .flags(if self.datasync {
                FsyncFlags::DATASYNC
            } else {
                FsyncFlags::empty()
            })
            .build()
            .into()
    }
}

impl OpCode for Unlink {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::UnlinkAt::new(Fd(libc::AT_FDCWD), self.path.as_ptr())
            .flags(if self.dir { libc::AT_REMOVEDIR } else { 0 })
            .build()
            .into()
    }
}

impl OpCode for CreateDir {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::MkDirAt::new(Fd(libc::AT_FDCWD), self.path.as_ptr())
            .mode(self.mode)
            .build()
            .into()
    }
}

impl OpCode for Rename {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::RenameAt::new(
            Fd(libc::AT_FDCWD),
            self.old_path.as_ptr(),
            Fd(libc::AT_FDCWD),
            self.new_path.as_ptr(),
        )
        .build()
        .into()
    }
}

impl OpCode for Symlink {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::SymlinkAt::new(
            Fd(libc::AT_FDCWD),
            self.source.as_ptr(),
            self.target.as_ptr(),
        )
        .build()
        .into()
    }
}

impl OpCode for HardLink {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::LinkAt::new(
            Fd(libc::AT_FDCWD),
            self.source.as_ptr(),
            Fd(libc::AT_FDCWD),
            self.target.as_ptr(),
        )
        .build()
        .into()
    }
}

impl OpCode for CreateSocket {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        if super::is_op_supported(opcode::Socket::CODE) {
            opcode::Socket::new(
                self.domain,
                self.socket_type | libc::SOCK_CLOEXEC,
                self.protocol,
            )
            .build()
            .into()
        } else {
            OpEntry::Blocking
        }
    }

    fn call_blocking(self: Pin<&mut Self>) -> io::Result<usize> {
        Ok(syscall!(libc::socket(
            self.domain,
            self.socket_type | libc::SOCK_CLOEXEC,
            self.protocol
        ))? as _)
    }
}

impl<S: AsFd> OpCode for ShutdownSocket<S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Shutdown::new(Fd(self.fd.as_fd().as_raw_fd()), self.how())
            .build()
            .into()
    }
}

impl OpCode for CloseSocket {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Close::new(Fd(self.fd.as_fd().as_raw_fd()))
            .build()
            .into()
    }
}

impl<S: AsFd> OpCode for Accept<S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        opcode::Accept::new(
            Fd(this.fd.as_fd().as_raw_fd()),
            unsafe { this.buffer.view_as::<libc::sockaddr>() },
            this.addr_len,
        )
        .flags(libc::SOCK_CLOEXEC)
        .build()
        .into()
    }

    unsafe fn set_result(self: Pin<&mut Self>, fd: usize) {
        // SAFETY: fd is a valid fd returned from kernel
        let fd = unsafe { OwnedFd::from_raw_fd(fd as _) };
        *self.project().accepted_fd = Some(fd);
    }
}

impl<S: AsFd> OpCode for Connect<S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        opcode::Connect::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.addr.as_ptr().cast(),
            self.addr.len(),
        )
        .build()
        .into()
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        let flags = self.flags;
        let slice = self.project().buffer.sys_slice_mut();
        opcode::Recv::new(Fd(fd), slice.ptr() as _, slice.len() as _)
            .flags(flags)
            .build()
            .into()
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.as_mut().set_msg();
        let this = self.project();
        opcode::RecvMsg::new(Fd(this.fd.as_fd().as_raw_fd()), this.msg)
            .flags(*this.flags as _)
            .build()
            .into()
    }
}

impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Send::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len() as _,
        )
        .flags(self.flags)
        .build()
        .into()
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.as_mut().set_msg();
        let this = self.project();
        opcode::SendMsg::new(Fd(this.fd.as_fd().as_raw_fd()), this.msg)
            .flags(*this.flags as _)
            .build()
            .into()
    }
}

struct RecvFromHeader<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddrStorage,
    pub(crate) msg: libc::msghdr,
    pub(crate) flags: i32,
    _p: PhantomPinned,
}

impl<S> RecvFromHeader<S> {
    pub fn new(fd: S, flags: i32) -> Self {
        Self {
            fd,
            addr: SockAddrStorage::zeroed(),
            msg: unsafe { std::mem::zeroed() },
            flags,
            _p: PhantomPinned,
        }
    }
}

impl<S: AsFd> RecvFromHeader<S> {
    pub fn create_entry(&mut self, slices: &mut [SysSlice]) -> OpEntry {
        self.msg.msg_name = &mut self.addr as *mut _ as _;
        self.msg.msg_namelen = self.addr.size_of() as _;
        self.msg.msg_iov = slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut self.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }

    pub fn into_addr(self) -> (SockAddrStorage, socklen_t) {
        (self.addr, self.msg.msg_namelen)
    }
}

pin_project! {
    /// Receive data and source address.
    pub struct RecvFrom<T: IoBufMut, S> {
        header: RecvFromHeader<S>,
        #[pin]
        buffer: T,
        slice: Option<SysSlice>,
    }
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            header: RecvFromHeader::new(fd, flags),
            buffer,
            slice: None,
        }
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        let slice = this.slice.insert(this.buffer.sys_slice_mut());
        this.header.create_entry(std::slice::from_mut(slice))
    }
}

impl<T: IoBufMut, S: AsFd> IntoInner for RecvFrom<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

pin_project! {
    /// Receive data and source address into vectored buffer.
    pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
        header: RecvFromHeader<S>,
        #[pin]
        buffer: T,
        slice: Vec<SysSlice>,
    }
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            header: RecvFromHeader::new(fd, flags),
            buffer,
            slice: vec![],
        }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        *this.slice = this.buffer.sys_slices_mut();
        this.header.create_entry(this.slice)
    }
}

impl<T: IoVectoredBufMut, S: AsFd> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

struct SendToHeader<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddr,
    pub(crate) msg: libc::msghdr,
    pub(crate) flags: i32,
    _p: PhantomPinned,
}

impl<S> SendToHeader<S> {
    pub fn new(fd: S, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            addr,
            msg: unsafe { std::mem::zeroed() },
            flags,
            _p: PhantomPinned,
        }
    }
}

impl<S: AsFd> SendToHeader<S> {
    pub fn create_entry(&mut self, slices: &mut [SysSlice]) -> OpEntry {
        self.msg.msg_name = self.addr.as_ptr() as _;
        self.msg.msg_namelen = self.addr.len();
        self.msg.msg_iov = slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &self.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

pin_project! {
    /// Send data to specified address.
    pub struct SendTo<T: IoBuf, S> {
        header: SendToHeader<S>,
        #[pin]
        buffer: T,
        slice: Option<SysSlice>,
    }
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            header: SendToHeader::new(fd, addr, flags),
            buffer,
            slice: None,
        }
    }
}

impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        let slice = this.slice.insert(this.buffer.as_ref().sys_slice());
        this.header.create_entry(std::slice::from_mut(slice))
    }
}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

pin_project! {
    /// Send data to specified address from vectored buffer.
    pub struct SendToVectored<T: IoVectoredBuf, S> {
        header: SendToHeader<S>,
        #[pin]
        buffer: T,
        slice: Vec<SysSlice>,
    }
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            header: SendToHeader::new(fd, addr, flags),
            buffer,
            slice: vec![],
        }
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let this = self.project();
        *this.slice = this.buffer.as_ref().sys_slices();
        this.header.create_entry(this.slice)
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.as_mut().set_msg();
        let this = self.project();
        opcode::RecvMsg::new(Fd(this.fd.as_fd().as_raw_fd()), this.msg)
            .flags(*this.flags as _)
            .build()
            .into()
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    fn create_entry(mut self: Pin<&mut Self>) -> OpEntry {
        self.as_mut().set_msg();
        let this = self.project();
        opcode::SendMsg::new(Fd(this.fd.as_fd().as_raw_fd()), this.msg)
            .flags(*this.flags as _)
            .build()
            .into()
    }
}

impl<S: AsFd> OpCode for PollOnce<S> {
    fn create_entry(self: Pin<&mut Self>) -> OpEntry {
        let flags = match self.interest {
            Interest::Readable => libc::POLLIN,
            Interest::Writable => libc::POLLOUT,
        };
        opcode::PollAdd::new(Fd(self.fd.as_fd().as_raw_fd()), flags as _)
            .build()
            .into()
    }
}

mod buf_ring {
    use std::{
        io,
        marker::PhantomPinned,
        os::fd::{AsFd, AsRawFd},
        pin::Pin,
        ptr,
    };

    use io_uring::{opcode, squeue::Flags, types::Fd};

    use super::OpCode;
    use crate::{BorrowedBuffer, BufferPool, OpEntry, TakeBuffer};

    /// Read a file at specified position into specified buffer.
    #[derive(Debug)]
    pub struct ReadManagedAt<S> {
        pub(crate) fd: S,
        pub(crate) offset: u64,
        buffer_group: u16,
        len: u32,
        _p: PhantomPinned,
    }

    impl<S> ReadManagedAt<S> {
        /// Create [`ReadManagedAt`].
        pub fn new(fd: S, offset: u64, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            Ok(Self {
                fd,
                offset,
                buffer_group: buffer_pool.buffer_group(),
                len: len.try_into().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
                })?,
                _p: PhantomPinned,
            })
        }
    }

    impl<S: AsFd> OpCode for ReadManagedAt<S> {
        fn create_entry(self: Pin<&mut Self>) -> OpEntry {
            let fd = Fd(self.fd.as_fd().as_raw_fd());
            let offset = self.offset;
            opcode::Read::new(fd, ptr::null_mut(), self.len)
                .offset(offset)
                .buf_group(self.buffer_group)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }
    }

    impl<S> TakeBuffer for ReadManagedAt<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            let result = result.inspect_err(|_| buffer_pool.reuse_buffer(buffer_id))?;
            // SAFETY: result is valid
            let res = unsafe { buffer_pool.get_buffer(buffer_id, result) };
            #[cfg(fusion)]
            let res = res.map(BorrowedBuffer::new_io_uring);
            res
        }
    }

    /// Receive data from remote.
    pub struct ReadManaged<S> {
        fd: S,
        buffer_group: u16,
        len: u32,
        _p: PhantomPinned,
    }

    impl<S> ReadManaged<S> {
        /// Create [`ReadManaged`].
        pub fn new(fd: S, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            Ok(Self {
                fd,
                buffer_group: buffer_pool.buffer_group(),
                len: len.try_into().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
                })?,
                _p: PhantomPinned,
            })
        }
    }

    impl<S: AsFd> OpCode for ReadManaged<S> {
        fn create_entry(self: Pin<&mut Self>) -> OpEntry {
            let fd = self.fd.as_fd().as_raw_fd();
            opcode::Read::new(Fd(fd), ptr::null_mut(), self.len)
                .buf_group(self.buffer_group)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }
    }

    impl<S> TakeBuffer for ReadManaged<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            let result = result.inspect_err(|_| buffer_pool.reuse_buffer(buffer_id))?;
            // SAFETY: result is valid
            let res = unsafe { buffer_pool.get_buffer(buffer_id, result) };
            #[cfg(fusion)]
            let res = res.map(BorrowedBuffer::new_io_uring);
            res
        }
    }

    /// Receive data from remote.
    pub struct RecvManaged<S> {
        fd: S,
        buffer_group: u16,
        len: u32,
        flags: i32,
        _p: PhantomPinned,
    }

    impl<S> RecvManaged<S> {
        /// Create [`RecvManaged`].
        pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            Ok(Self {
                fd,
                buffer_group: buffer_pool.buffer_group(),
                len: len.try_into().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
                })?,
                flags,
                _p: PhantomPinned,
            })
        }
    }

    impl<S: AsFd> OpCode for RecvManaged<S> {
        fn create_entry(self: Pin<&mut Self>) -> OpEntry {
            let fd = self.fd.as_fd().as_raw_fd();
            opcode::Recv::new(Fd(fd), ptr::null_mut(), self.len)
                .flags(self.flags)
                .buf_group(self.buffer_group)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }
    }

    impl<S> TakeBuffer for RecvManaged<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            let result = result.inspect_err(|_| buffer_pool.reuse_buffer(buffer_id))?;
            // SAFETY: result is valid
            let res = unsafe { buffer_pool.get_buffer(buffer_id, result) };
            #[cfg(fusion)]
            let res = res.map(BorrowedBuffer::new_io_uring);
            res
        }
    }
}

pub use buf_ring::{ReadManaged, ReadManagedAt, RecvManaged};
