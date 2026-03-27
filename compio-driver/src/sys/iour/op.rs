use std::{
    collections::VecDeque,
    ffi::CString,
    io,
    os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd, OwnedFd},
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use io_uring::{
    opcode,
    types::{Fd, FsyncFlags},
};
use socket2::{SockAddr, SockAddrStorage, Socket as Socket2, socklen_t};

use super::OpCode;
pub use crate::sys::unix_op::*;
use crate::{Extra, OpEntry, op::*, sys_slice::*, syscall};

unsafe impl<D, F> OpCode for Asyncify<F, D>
where
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        self.data = Some(data);
        res
    }
}

unsafe impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd);
        self.data = Some(data);
        res
    }
}

unsafe impl<S1, S2, D, F> OpCode for AsyncifyFd2<S1, S2, F, D>
where
    S1: std::marker::Sync,
    S2: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd1, &self.fd2);
        self.data = Some(data);
        res
    }
}

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::OpenAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .flags(self.flags | libc::O_CLOEXEC)
            .mode(self.mode)
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }

    unsafe fn set_result(
        &mut self,
        _control: &mut Self::Control,
        res: &io::Result<usize>,
        _: &Extra,
    ) {
        if let Ok(fd) = res {
            // SAFETY: fd is a valid fd returned from kernel
            let fd = unsafe { OwnedFd::from_raw_fd(*fd as _) };
            self.opened_fd = Some(fd);
        }
    }
}

unsafe impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Close::new(Fd(self.fd.as_fd().as_raw_fd()))
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Ftruncate::new(Fd(self.fd.as_fd().as_raw_fd()), self.size)
            .build()
            .into()
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        self.call()
    }
}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
    pub(crate) stat: Statx,
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

unsafe impl<S: AsFd> OpCode for FileStat<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        static EMPTY_NAME: &[u8] = b"\0";
        opcode::Statx::new(
            Fd(self.fd.as_fd().as_fd().as_raw_fd()),
            EMPTY_NAME.as_ptr().cast(),
            &raw mut self.stat as _,
        )
        .flags(libc::AT_EMPTY_PATH)
        .mask(statx_mask())
        .build()
        .into()
    }

    #[cfg(gnulinux)]
    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        static EMPTY_NAME: &[u8] = b"\0";
        let res = syscall!(libc::statx(
            self.fd.as_fd().as_raw_fd(),
            EMPTY_NAME.as_ptr().cast(),
            libc::AT_EMPTY_PATH,
            statx_mask(),
            &raw mut self.stat as _
        ))?;
        Ok(res as _)
    }

    #[cfg(not(gnulinux))]
    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        let mut stat = unsafe { std::mem::zeroed() };
        let res = syscall!(libc::fstat(self.fd.as_fd().as_raw_fd(), &mut stat))?;
        self.stat = stat_to_statx(stat);
        Ok(res as _)
    }
}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

/// Get metadata from path.
pub struct PathStat<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) stat: Statx,
    pub(crate) follow_symlink: bool,
}

impl<S: AsFd> PathStat<S> {
    /// Create [`PathStat`].
    pub fn new(dirfd: S, path: CString, follow_symlink: bool) -> Self {
        Self {
            dirfd,
            path,
            stat: unsafe { std::mem::zeroed() },
            follow_symlink,
        }
    }
}

unsafe impl<S: AsFd> OpCode for PathStat<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        opcode::Statx::new(
            Fd(self.dirfd.as_fd().as_raw_fd()),
            self.path.as_ptr(),
            &raw mut self.stat as _,
        )
        .flags(flags)
        .mask(statx_mask())
        .build()
        .into()
    }

    #[cfg(gnulinux)]
    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        let res = syscall!(libc::statx(
            self.dirfd.as_fd().as_raw_fd(),
            self.path.as_ptr(),
            flags,
            statx_mask(),
            &raw mut self.stat
        ))?;
        Ok(res as _)
    }

    #[cfg(not(gnulinux))]
    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        let mut flags = libc::AT_EMPTY_PATH;
        if !self.follow_symlink {
            flags |= libc::AT_SYMLINK_NOFOLLOW;
        }
        let mut stat = unsafe { std::mem::zeroed() };
        let res = syscall!(libc::fstatat(
            self.dirfd.as_fd().as_raw_fd(),
            self.path.as_ptr(),
            &mut stat,
            flags
        ))?;
        self.stat = stat_to_statx(stat);
        Ok(res as _)
    }
}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        statx_to_stat(self.stat)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let fd = Fd(self.fd.as_fd().as_raw_fd());
        let slice = self.buffer.sys_slice_mut();
        opcode::Read::new(
            fd,
            slice.ptr() as _,
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.slices = self.buffer.sys_slices_mut();
        opcode::Readv::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.slices.as_ptr() as _,
            self.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Write::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.slices = self.buffer.sys_slices();
        opcode::Writev::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.slices.as_ptr() as _,
            self.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .offset(self.offset)
        .build()
        .into()
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.buffer.sys_slice_mut();
        opcode::Read::new(
            Fd(fd),
            slice.ptr() as _,
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.slices = self.buffer.sys_slices_mut();
        opcode::Readv::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.slices.as_ptr() as _,
            self.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Write::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.slices = self.buffer.sys_slices();
        opcode::Writev::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.slices.as_ptr() as _,
            self.slices.len().try_into().unwrap_or(u32::MAX),
        )
        .build()
        .into()
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
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

unsafe impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::UnlinkAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .flags(if self.dir { libc::AT_REMOVEDIR } else { 0 })
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::MkDirAt::new(Fd(self.dirfd.as_fd().as_raw_fd()), self.path.as_ptr())
            .mode(self.mode)
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::RenameAt::new(
            Fd(self.old_dirfd.as_fd().as_raw_fd()),
            self.old_path.as_ptr(),
            Fd(self.new_dirfd.as_fd().as_raw_fd()),
            self.new_path.as_ptr(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::SymlinkAt::new(
            Fd(self.dirfd.as_fd().as_raw_fd()),
            self.source.as_ptr(),
            self.target.as_ptr(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::LinkAt::new(
            Fd(self.source_dirfd.as_fd().as_raw_fd()),
            self.source.as_ptr(),
            Fd(self.target_dirfd.as_fd().as_raw_fd()),
            self.target.as_ptr(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl OpCode for CreateSocket {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Socket::new(
            self.domain,
            self.socket_type | libc::SOCK_CLOEXEC,
            self.protocol,
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        Ok(syscall!(libc::socket(
            self.domain,
            self.socket_type | libc::SOCK_CLOEXEC,
            self.protocol
        ))? as _)
    }

    unsafe fn set_result(
        &mut self,
        _control: &mut Self::Control,
        res: &io::Result<usize>,
        _: &Extra,
    ) {
        if let Ok(fd) = res {
            // SAFETY: fd is a valid fd returned from kernel
            let fd = unsafe { Socket2::from_raw_fd(*fd as _) };
            self.opened_fd = Some(fd);
        }
    }
}

unsafe impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Shutdown::new(Fd(self.fd.as_fd().as_raw_fd()), self.how())
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl OpCode for CloseSocket {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Close::new(Fd(self.fd.as_fd().as_raw_fd()))
            .build()
            .into()
    }

    fn call_blocking(&mut self, control: &mut Self::Control) -> io::Result<usize> {
        self.call(control)
    }
}

unsafe impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Accept::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            unsafe { self.buffer.view_as::<libc::sockaddr>() },
            &raw mut self.addr_len,
        )
        .flags(libc::SOCK_CLOEXEC)
        .build()
        .into()
    }

    unsafe fn set_result(
        &mut self,
        _control: &mut Self::Control,
        res: &io::Result<usize>,
        _: &Extra,
    ) {
        if let Ok(fd) = res {
            // SAFETY: fd is a valid fd returned from kernel
            let fd = unsafe { Socket2::from_raw_fd(*fd as _) };
            self.accepted_fd = Some(fd);
        }
    }
}

struct AcceptMultishotResult {
    res: io::Result<Socket2>,
    extra: crate::Extra,
}

impl AcceptMultishotResult {
    pub unsafe fn new(res: io::Result<usize>, extra: crate::Extra) -> Self {
        Self {
            res: res.map(|fd| unsafe { Socket2::from_raw_fd(fd as _) }),
            extra,
        }
    }

    pub fn into_result(self) -> BufResult<usize, crate::Extra> {
        BufResult(self.res.map(|fd| fd.into_raw_fd() as _), self.extra)
    }
}

/// Accept multiple connections.
pub struct AcceptMulti<S> {
    pub(crate) op: Accept<S>,
    multishots: VecDeque<AcceptMultishotResult>,
}

impl<S> AcceptMulti<S> {
    /// Create [`AcceptMulti`].
    pub fn new(fd: S) -> Self {
        Self {
            op: Accept::new(fd),
            multishots: VecDeque::new(),
        }
    }
}

unsafe impl<S: AsFd> OpCode for AcceptMulti<S> {
    type Control = <Accept<S> as OpCode>::Control;

    unsafe fn init(&mut self) -> Self::Control {
        unsafe { self.op.init() }
    }

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::AcceptMulti::new(Fd(self.op.fd.as_fd().as_raw_fd()))
            .flags(libc::SOCK_CLOEXEC)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        res: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, res, extra) }
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.multishots
            .push_back(unsafe { AcceptMultishotResult::new(res, extra) });
    }

    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        self.multishots.pop_front().map(|res| res.into_result())
    }
}

impl<S> IntoInner for AcceptMulti<S> {
    type Inner = Socket2;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner().0
    }
}

unsafe impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Connect::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.addr.as_ptr().cast(),
            self.addr.len(),
        )
        .build()
        .into()
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let fd = self.fd.as_fd().as_raw_fd();
        let flags = self.flags;
        let slice = self.buffer.sys_slice_mut();
        opcode::Recv::new(
            Fd(fd),
            slice.ptr() as _,
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .flags(flags)
        .build()
        .into()
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.set_msg(control);
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut self.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.buffer.as_init();
        opcode::Send::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .flags(self.flags)
        .build()
        .into()
    }
}

/// Zerocopy [`Send`].
pub struct SendZc<T: IoBuf, S> {
    pub(crate) op: Send<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

impl<T: IoBuf, S> SendZc<T, S> {
    /// Create [`SendZc`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            op: Send::new(fd, buffer, flags),
            res: None,
        }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendZc<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.op.buffer.as_init();
        opcode::SendZc::new(
            Fd(self.op.fd.as_fd().as_raw_fd()),
            slice.as_ptr(),
            slice.len().try_into().unwrap_or(u32::MAX),
        )
        .flags(self.op.flags)
        .build()
        .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoBuf, S> IntoInner for SendZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.set_msg(control);
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut self.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

/// Zerocopy [`SendVectored`].
pub struct SendVectoredZc<T: IoVectoredBuf, S> {
    pub(crate) op: SendVectored<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

impl<T: IoVectoredBuf, S> SendVectoredZc<T, S> {
    /// Create [`SendVectoredZc`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            op: SendVectored::new(fd, buffer, flags),
            res: None,
        }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectoredZc<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.set_msg(control);
        opcode::SendMsgZc::new(Fd(self.op.fd.as_fd().as_raw_fd()), &self.op.msg)
            .flags(self.op.flags as _)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendVectoredZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

struct RecvFromHeader<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddrStorage,
    pub(crate) msg: libc::msghdr,
    pub(crate) flags: i32,
}

impl<S> RecvFromHeader<S> {
    pub fn new(fd: S, flags: i32) -> Self {
        Self {
            fd,
            addr: SockAddrStorage::zeroed(),
            msg: unsafe { std::mem::zeroed() },
            flags,
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

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    header: RecvFromHeader<S>,
    buffer: T,
    slice: Option<SysSlice>,
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

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.slice.insert(self.buffer.sys_slice_mut());
        self.header.create_entry(std::slice::from_mut(slice))
    }
}

impl<T: IoBufMut, S: AsFd> IntoInner for RecvFrom<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        let (addr, addr_len) = self.header.into_addr();
        (self.buffer, addr, addr_len)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    header: RecvFromHeader<S>,
    buffer: T,
    slice: Vec<SysSlice>,
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

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.slice = self.buffer.sys_slices_mut();
        self.header.create_entry(&mut self.slice)
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
}

impl<S> SendToHeader<S> {
    pub fn new(fd: S, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            addr,
            msg: unsafe { std::mem::zeroed() },
            flags,
        }
    }
}

impl<S: AsFd> SendToHeader<S> {
    pub fn set_msg(&mut self, slices: &mut [SysSlice]) {
        self.msg.msg_name = self.addr.as_ptr() as _;
        self.msg.msg_namelen = self.addr.len();
        self.msg.msg_iov = slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    header: SendToHeader<S>,
    buffer: T,
    slice: Option<SysSlice>,
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

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.slice.insert(self.buffer.sys_slice());
        self.header.set_msg(std::slice::from_mut(slice));
        opcode::SendMsg::new(Fd(self.header.fd.as_fd().as_raw_fd()), &self.header.msg)
            .flags(self.header.flags as _)
            .build()
            .into()
    }
}

impl<T: IoBuf, S> IntoInner for SendTo<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Zerocopy [`SendTo`].
pub struct SendToZc<T: IoBuf, S: AsFd> {
    pub(crate) op: SendTo<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

impl<T: IoBuf, S: AsFd> SendToZc<T, S> {
    /// Create [`SendToZc`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            op: SendTo::new(fd, buffer, addr, flags),
            res: None,
        }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendToZc<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let slice = self.op.slice.insert(self.op.buffer.sys_slice());
        self.op.header.set_msg(std::slice::from_mut(slice));
        opcode::SendMsgZc::new(
            Fd(self.op.header.fd.as_fd().as_raw_fd()),
            &self.op.header.msg,
        )
        .flags(self.op.header.flags as _)
        .build()
        .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoBuf, S: AsFd> IntoInner for SendToZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    header: SendToHeader<S>,
    buffer: T,
    slice: Vec<SysSlice>,
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

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.slice = self.buffer.sys_slices();
        self.header.set_msg(&mut self.slice);
        opcode::SendMsg::new(Fd(self.header.fd.as_fd().as_raw_fd()), &self.header.msg)
            .flags(self.header.flags as _)
            .build()
            .into()
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

/// Zerocopy [`SendToVectored`].
pub struct SendToVectoredZc<T: IoVectoredBuf, S: AsFd> {
    pub(crate) op: SendToVectored<T, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectoredZc<T, S> {
    /// Create [`SendToVectoredZc`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            op: SendToVectored::new(fd, buffer, addr, flags),
            res: None,
        }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectoredZc<T, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        self.op.slice = self.op.buffer.sys_slices();
        self.op.header.set_msg(&mut self.op.slice);
        opcode::SendMsgZc::new(
            Fd(self.op.header.fd.as_fd().as_raw_fd()),
            &self.op.header.msg,
        )
        .flags(self.op.header.flags as _)
        .build()
        .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}

impl<T: IoVectoredBuf, S: AsFd> IntoInner for SendToVectoredZc<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.set_msg(control);
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut self.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.set_msg(control);
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut self.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }
}

/// Zerocopy [`SendMsg`].
pub struct SendMsgZc<T: IoVectoredBuf, C: IoBuf, S> {
    pub(crate) op: SendMsg<T, C, S>,
    pub(crate) res: Option<BufResult<usize, crate::Extra>>,
}

impl<T: IoVectoredBuf, C: IoBuf, S> SendMsgZc<T, C, S> {
    /// Create [`SendMsgZc`].
    pub fn new(fd: S, buffer: T, control: C, addr: Option<SockAddr>, flags: i32) -> Self {
        Self {
            op: SendMsg::new(fd, buffer, control, addr, flags),
            res: None,
        }
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsgZc<T, C, S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.set_msg(control);
        opcode::SendMsgZc::new(Fd(self.op.fd.as_fd().as_raw_fd()), &self.op.msg)
            .flags(self.op.flags as _)
            .build()
            .into()
    }

    fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
        self.op.create_entry(control)
    }

    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        res: io::Result<usize>,
        extra: crate::Extra,
    ) {
        self.res.replace(BufResult(res, extra));
    }

    fn pop_multishot(
        &mut self,
        _control: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::Extra>> {
        self.res.take()
    }
}
impl<T: IoVectoredBuf, C: IoBuf, S> IntoInner for SendMsgZc<T, C, S> {
    type Inner = (T, C);

    fn into_inner(self) -> Self::Inner {
        self.op.into_inner()
    }
}

unsafe impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        let flags = match self.interest {
            Interest::Readable => libc::POLLIN,
            Interest::Writable => libc::POLLOUT,
        };
        opcode::PollAdd::new(Fd(self.fd.as_fd().as_raw_fd()), flags as _)
            .build()
            .into()
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        opcode::Splice::new(
            Fd(self.fd_in.as_fd().as_raw_fd()),
            self.offset_in,
            Fd(self.fd_out.as_fd().as_raw_fd()),
            self.offset_out,
            self.len.try_into().unwrap_or(u32::MAX),
        )
        .flags(self.flags)
        .build()
        .into()
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> io::Result<usize> {
        let mut offset_in = self.offset_in;
        let mut offset_out = self.offset_out;
        let offset_in_ptr = if offset_in < 0 {
            std::ptr::null_mut()
        } else {
            &mut offset_in
        };
        let offset_out_ptr = if offset_out < 0 {
            std::ptr::null_mut()
        } else {
            &mut offset_out
        };
        Ok(syscall!(libc::splice(
            self.fd_in.as_fd().as_raw_fd(),
            offset_in_ptr,
            self.fd_out.as_fd().as_raw_fd(),
            offset_out_ptr,
            self.len,
            self.flags,
        ))? as _)
    }
}

mod buf_ring {
    use std::{
        collections::VecDeque,
        io,
        os::fd::{AsFd, AsRawFd},
        ptr,
    };

    use compio_buf::BufResult;
    use io_uring::{opcode, squeue::Flags, types::Fd};
    use socket2::{SockAddr, SockAddrStorage, socklen_t};

    use super::{Extra, OpCode};
    use crate::{
        BorrowedBuffer, BufferPool, IoUringBufferPool, IoUringOwnedBuffer, OpEntry, TakeBuffer,
    };

    pub(crate) fn take_buffer(
        buffer_pool: &BufferPool,
        result: io::Result<usize>,
        buffer_id: u16,
    ) -> io::Result<BorrowedBuffer<'_>> {
        #[cfg(fusion)]
        let buffer_pool = buffer_pool.as_io_uring();
        let result = result.inspect_err(|_| buffer_pool.reuse_buffer(buffer_id))?;
        // SAFETY: result is valid
        let buffer = unsafe { buffer_pool.get_buffer(buffer_id, result) };
        let res = unsafe { buffer_pool.create_proxy(buffer, result) }?;
        #[cfg(fusion)]
        let res = BorrowedBuffer::new_io_uring(res);
        Ok(res)
    }

    /// Read a file at specified position into specified buffer.
    pub struct ReadManagedAt<S> {
        pub(crate) fd: S,
        pub(crate) offset: u64,
        buffer_group: u16,
        len: u32,
        pool: IoUringBufferPool,
        buffer: Option<IoUringOwnedBuffer>,
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
                pool: buffer_pool.clone(),
                buffer: None,
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for ReadManagedAt<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            let fd = Fd(self.fd.as_fd().as_raw_fd());
            let offset = self.offset;
            opcode::Read::new(fd, ptr::null_mut(), self.len)
                .offset(offset)
                .buf_group(self.buffer_group)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }

        unsafe fn set_result(
            &mut self,
            _control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &Extra,
        ) {
            if let Ok(buffer_id) = extra.buffer_id() {
                self.buffer.replace(unsafe {
                    self.pool.get_buffer(buffer_id, *res.as_ref().unwrap_or(&0))
                });
            }
        }
    }

    impl<S> TakeBuffer for ReadManagedAt<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            mut self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            self.buffer.take().map(|buf| buf.leak());
            take_buffer(buffer_pool, result, buffer_id)
        }
    }

    /// Read a file.
    pub struct ReadManaged<S> {
        fd: S,
        buffer_group: u16,
        len: u32,
        pool: IoUringBufferPool,
        buffer: Option<IoUringOwnedBuffer>,
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
                pool: buffer_pool.clone(),
                buffer: None,
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for ReadManaged<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            let fd = self.fd.as_fd().as_raw_fd();
            opcode::Read::new(Fd(fd), ptr::null_mut(), self.len)
                .buf_group(self.buffer_group)
                .offset(u64::MAX)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }

        unsafe fn set_result(
            &mut self,
            _control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &Extra,
        ) {
            if let Ok(buffer_id) = extra.buffer_id() {
                self.buffer.replace(unsafe {
                    self.pool.get_buffer(buffer_id, *res.as_ref().unwrap_or(&0))
                });
            }
        }
    }

    impl<S> TakeBuffer for ReadManaged<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            mut self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            self.buffer.take().map(|buf| buf.leak());
            take_buffer(buffer_pool, result, buffer_id)
        }
    }

    /// Receive data from remote.
    pub struct RecvManaged<S> {
        fd: S,
        buffer_group: u16,
        len: u32,
        flags: i32,
        pool: IoUringBufferPool,
        buffer: Option<IoUringOwnedBuffer>,
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
                pool: buffer_pool.clone(),
                buffer: None,
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for RecvManaged<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            let fd = self.fd.as_fd().as_raw_fd();
            opcode::Recv::new(Fd(fd), ptr::null_mut(), self.len)
                .flags(self.flags)
                .buf_group(self.buffer_group)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }

        unsafe fn set_result(
            &mut self,
            _control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &Extra,
        ) {
            if let Ok(buffer_id) = extra.buffer_id() {
                self.buffer.replace(unsafe {
                    self.pool.get_buffer(buffer_id, *res.as_ref().unwrap_or(&0))
                });
            }
        }
    }

    impl<S> TakeBuffer for RecvManaged<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            mut self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            self.buffer.take().map(|buf| buf.leak());
            take_buffer(buffer_pool, result, buffer_id)
        }
    }

    /// Receive data and source address into managed buffer.
    pub struct RecvFromManaged<S> {
        fd: S,
        buffer_group: u16,
        flags: i32,
        addr: SockAddrStorage,
        addr_len: socklen_t,
        iovec: libc::iovec,
        msg: libc::msghdr,
        pool: IoUringBufferPool,
        buffer: Option<IoUringOwnedBuffer>,
    }

    impl<S> RecvFromManaged<S> {
        /// Create [`RecvFromManaged`].
        pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            let len: u32 = len.try_into().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "required length too long")
            })?;
            let addr = SockAddrStorage::zeroed();
            Ok(Self {
                fd,
                buffer_group: buffer_pool.buffer_group(),
                flags,
                addr_len: addr.size_of() as _,
                addr,
                iovec: libc::iovec {
                    iov_base: ptr::null_mut(),
                    iov_len: len as _,
                },
                msg: unsafe { std::mem::zeroed() },
                pool: buffer_pool.clone(),
                buffer: None,
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for RecvFromManaged<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            self.msg.msg_name = &raw mut self.addr as *mut _ as _;
            self.msg.msg_namelen = self.addr_len;
            self.msg.msg_iov = &raw mut self.iovec as *const _ as *mut _;
            self.msg.msg_iovlen = 1;
            opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &raw mut self.msg)
                .flags(self.flags as _)
                .buf_group(self.buffer_group)
                .build()
                .flags(Flags::BUFFER_SELECT)
                .into()
        }

        unsafe fn set_result(
            &mut self,
            _control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &Extra,
        ) {
            if let Ok(buffer_id) = extra.buffer_id() {
                self.buffer.replace(unsafe {
                    self.pool.get_buffer(buffer_id, *res.as_ref().unwrap_or(&0))
                });
            }
        }
    }

    impl<S> TakeBuffer for RecvFromManaged<S> {
        type Buffer<'a> = (BorrowedBuffer<'a>, Option<SockAddr>);
        type BufferPool = BufferPool;

        fn take_buffer(
            mut self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            #[cfg(fusion)]
            let buffer_pool = buffer_pool.as_io_uring();
            let result = result.inspect_err(|_| buffer_pool.reuse_buffer(buffer_id))?;
            let addr =
                (self.addr_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.addr_len) });
            // SAFETY: result is valid
            let buffer = self
                .buffer
                .take()
                .unwrap_or_else(|| unsafe { buffer_pool.get_buffer(buffer_id, result) });
            let buffer = unsafe { buffer_pool.create_proxy(buffer, result) }?;
            #[cfg(fusion)]
            let buffer = BorrowedBuffer::new_io_uring(buffer);
            Ok((buffer, addr))
        }
    }

    struct MultishotResult {
        result: io::Result<usize>,
        extra: Extra,
        buffer: Option<IoUringOwnedBuffer>,
    }

    impl MultishotResult {
        pub fn new(result: io::Result<usize>, extra: Extra, pool: &IoUringBufferPool) -> Self {
            let buffer = extra
                .buffer_id()
                .map(|buffer_id| unsafe {
                    pool.get_buffer(buffer_id, *result.as_ref().unwrap_or(&0))
                })
                .ok();
            Self {
                result,
                extra,
                buffer,
            }
        }

        pub fn leak(&mut self) {
            self.buffer.take().map(|buf| buf.leak());
        }
    }

    /// Read a file at specified position into multiple managed buffers.
    pub struct ReadMultiAt<S> {
        inner: ReadManagedAt<S>,
        multishots: VecDeque<MultishotResult>,
    }

    impl<S> ReadMultiAt<S> {
        /// Create [`ReadMultiAt`].
        pub fn new(fd: S, offset: u64, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
            Ok(Self {
                inner: ReadManagedAt::new(fd, offset, buffer_pool, len)?,
                multishots: VecDeque::new(),
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for ReadMultiAt<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            let fd = self.inner.fd.as_fd().as_raw_fd();
            opcode::ReadMulti::new(Fd(fd), self.inner.len, self.inner.buffer_group)
                .offset(self.inner.offset)
                .build()
                .into()
        }

        fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
            self.inner.create_entry(control)
        }

        unsafe fn set_result(
            &mut self,
            control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &crate::Extra,
        ) {
            unsafe { self.inner.set_result(control, res, extra) }
        }

        unsafe fn push_multishot(
            &mut self,
            _: &mut Self::Control,
            res: io::Result<usize>,
            extra: crate::Extra,
        ) {
            self.multishots
                .push_back(MultishotResult::new(res, extra, &self.inner.pool));
        }

        fn pop_multishot(
            &mut self,
            _: &mut Self::Control,
        ) -> Option<BufResult<usize, crate::sys::Extra>> {
            self.multishots.pop_front().map(|mut proxy| {
                proxy.leak();
                BufResult(proxy.result, proxy.extra)
            })
        }
    }

    impl<S> TakeBuffer for ReadMultiAt<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            self.inner.take_buffer(buffer_pool, result, buffer_id)
        }
    }

    /// Read a file into multiple managed buffers.
    pub struct ReadMulti<S> {
        inner: ReadManaged<S>,
        multishots: VecDeque<MultishotResult>,
    }

    impl<S> ReadMulti<S> {
        /// Create [`ReadMulti`].
        pub fn new(fd: S, buffer_pool: &BufferPool, len: usize) -> io::Result<Self> {
            Ok(Self {
                inner: ReadManaged::new(fd, buffer_pool, len)?,
                multishots: VecDeque::new(),
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for ReadMulti<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            let fd = self.inner.fd.as_fd().as_raw_fd();
            opcode::ReadMulti::new(Fd(fd), self.inner.len, self.inner.buffer_group)
                .offset(u64::MAX)
                .build()
                .into()
        }

        fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
            self.inner.create_entry(control)
        }

        unsafe fn set_result(
            &mut self,
            control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &crate::Extra,
        ) {
            unsafe { self.inner.set_result(control, res, extra) }
        }

        unsafe fn push_multishot(
            &mut self,
            _: &mut Self::Control,
            res: io::Result<usize>,
            extra: crate::Extra,
        ) {
            self.multishots
                .push_back(MultishotResult::new(res, extra, &self.inner.pool));
        }

        fn pop_multishot(
            &mut self,
            _: &mut Self::Control,
        ) -> Option<BufResult<usize, crate::sys::Extra>> {
            self.multishots.pop_front().map(|mut proxy| {
                proxy.leak();
                BufResult(proxy.result, proxy.extra)
            })
        }
    }

    impl<S> TakeBuffer for ReadMulti<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            self.inner.take_buffer(buffer_pool, result, buffer_id)
        }
    }

    /// Receive data from remote into multiple managed buffers.
    pub struct RecvMulti<S> {
        inner: RecvManaged<S>,
        multishots: VecDeque<MultishotResult>,
    }

    impl<S> RecvMulti<S> {
        /// Create [`RecvMulti`].
        pub fn new(fd: S, buffer_pool: &BufferPool, len: usize, flags: i32) -> io::Result<Self> {
            Ok(Self {
                inner: RecvManaged::new(fd, buffer_pool, len, flags)?,
                multishots: VecDeque::new(),
            })
        }
    }

    unsafe impl<S: AsFd> OpCode for RecvMulti<S> {
        type Control = ();

        unsafe fn init(&mut self) -> Self::Control {}

        fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
            let fd = self.inner.fd.as_fd().as_raw_fd();
            opcode::RecvMulti::new(Fd(fd), self.inner.buffer_group)
                .flags(self.inner.flags)
                .build()
                .into()
        }

        fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
            self.inner.create_entry(control)
        }

        unsafe fn set_result(
            &mut self,
            control: &mut Self::Control,
            res: &io::Result<usize>,
            extra: &crate::Extra,
        ) {
            unsafe { self.inner.set_result(control, res, extra) }
        }

        unsafe fn push_multishot(
            &mut self,
            _: &mut Self::Control,
            res: io::Result<usize>,
            extra: crate::Extra,
        ) {
            self.multishots
                .push_back(MultishotResult::new(res, extra, &self.inner.pool));
        }

        fn pop_multishot(
            &mut self,
            _: &mut Self::Control,
        ) -> Option<BufResult<usize, crate::sys::Extra>> {
            self.multishots.pop_front().map(|mut proxy| {
                proxy.leak();
                BufResult(proxy.result, proxy.extra)
            })
        }
    }

    impl<S> TakeBuffer for RecvMulti<S> {
        type Buffer<'a> = BorrowedBuffer<'a>;
        type BufferPool = BufferPool;

        fn take_buffer(
            self,
            buffer_pool: &Self::BufferPool,
            result: io::Result<usize>,
            buffer_id: u16,
        ) -> io::Result<Self::Buffer<'_>> {
            self.inner.take_buffer(buffer_pool, result, buffer_id)
        }
    }
}

pub(crate) use buf_ring::take_buffer;
pub use buf_ring::{
    ReadManaged, ReadManagedAt, ReadMulti, ReadMultiAt, RecvFromManaged, RecvManaged, RecvMulti,
};
