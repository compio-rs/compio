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
use socket2::{SockAddr, SockAddrStorage, Socket as Socket2};

mod managed;

pub use self::managed::{
    ReadManaged, ReadManagedAt, ReadMulti, ReadMultiAt, RecvFromManaged, RecvFromMulti,
    RecvFromMultiResult, RecvManaged, RecvMsgManaged, RecvMsgMulti, RecvMsgMultiResult, RecvMulti,
};
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
    type Control = ReadVectoredAtControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Readv::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
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
    type Control = WriteVectoredAtControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Writev::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
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
    type Control = ReadVectoredControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Readv::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
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
    type Control = WriteVectoredControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::Writev::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            control.slices.as_ptr() as _,
            control.slices.len().try_into().unwrap_or(u32::MAX),
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

unsafe impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Bind::new(
            Fd(self.fd.as_fd().as_raw_fd()),
            self.addr.as_ptr().cast(),
            self.addr.len(),
        )
        .build()
        .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        syscall!(libc::bind(
            self.fd.as_fd().as_raw_fd(),
            self.addr.as_ptr().cast(),
            self.addr.len()
        ))
        .map(|res| res as _)
    }
}

unsafe impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Listen::new(Fd(self.fd.as_fd().as_raw_fd()), self.backlog)
            .build()
            .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        syscall!(libc::listen(self.fd.as_fd().as_raw_fd(), self.backlog)).map(|res| res as _)
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
    type Control = RecvVectoredControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut control.msg)
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
    type Control = SendVectoredControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &control.msg)
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
    type Control = SendVectoredControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.op.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.fd.as_fd().as_raw_fd()), &control.msg)
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
    pub(crate) flags: i32,
    pub(crate) name_len: libc::socklen_t,
}

impl<S> RecvFromHeader<S> {
    pub fn new(fd: S, flags: i32) -> Self {
        Self {
            fd,
            addr: SockAddrStorage::zeroed(),
            flags,
            name_len: 0,
        }
    }
}

impl<S: AsFd> RecvFromHeader<S> {
    pub fn create_control(&mut self, mut slices: Vec<SysSlice>) -> RecvMsgControl {
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_name = &mut self.addr as *mut _ as _;
        msg.msg_namelen = self.addr.size_of() as _;
        msg.msg_iov = slices.as_mut_ptr().cast();
        msg.msg_iovlen = slices.len() as _;

        RecvMsgControl { msg, slices }
    }

    pub fn create_entry(&mut self, control: &mut RecvMsgControl) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut control.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }

    pub fn set_result(&mut self, control: &mut RecvMsgControl) {
        self.name_len = control.msg.msg_namelen;
    }

    pub fn into_addr(self) -> Option<SockAddr> {
        (self.name_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.name_len) })
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    header: RecvFromHeader<S>,
    buffer: T,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            header: RecvFromHeader::new(fd, flags),
            buffer,
        }
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.header
            .create_control(vec![self.buffer.sys_slice_mut()])
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.header.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.header.set_result(control);
    }
}

impl<T: IoBufMut, S: AsFd> IntoInner for RecvFrom<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = self.header.into_addr();
        (self.buffer, addr)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    header: RecvFromHeader<S>,
    buffer: T,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            header: RecvFromHeader::new(fd, flags),
            buffer,
        }
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.header.create_control(self.buffer.sys_slices_mut())
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        self.header.create_entry(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.header.set_result(control);
    }
}

impl<T: IoVectoredBufMut, S: AsFd> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = self.header.into_addr();
        (self.buffer, addr)
    }
}

struct SendToHeader<S> {
    pub(crate) fd: S,
    pub(crate) addr: SockAddr,
    pub(crate) flags: i32,
}

impl<S> SendToHeader<S> {
    pub fn new(fd: S, addr: SockAddr, flags: i32) -> Self {
        Self { fd, addr, flags }
    }
}

impl<S: AsFd> SendToHeader<S> {
    pub fn create_control(&mut self, mut slices: Vec<SysSlice>) -> SendMsgControl {
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_name = self.addr.as_ptr() as _;
        msg.msg_namelen = self.addr.len();
        msg.msg_iov = slices.as_mut_ptr() as _;
        msg.msg_iovlen = slices.len() as _;
        SendMsgControl { msg, slices }
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    header: SendToHeader<S>,
    buffer: T,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            header: SendToHeader::new(fd, addr, flags),
            buffer,
        }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.header.create_control(vec![self.buffer.sys_slice()])
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.header.fd.as_fd().as_raw_fd()), &control.msg)
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
    type Control = SendMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        unsafe { self.op.init() }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.header.fd.as_fd().as_raw_fd()), &control.msg)
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
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            header: SendToHeader::new(fd, addr, flags),
            buffer,
        }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.header.create_control(self.buffer.sys_slices())
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.header.fd.as_fd().as_raw_fd()), &control.msg)
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
    type Control = SendMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        unsafe { self.op.init() }
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.header.fd.as_fd().as_raw_fd()), &control.msg)
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
    type Control = RecvMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::RecvMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &mut control.msg)
            .flags(self.flags as _)
            .build()
            .into()
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.update_control(control);
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsg::new(Fd(self.fd.as_fd().as_raw_fd()), &control.msg)
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
    type Control = SendMsgControl;

    unsafe fn init(&mut self) -> Self::Control {
        self.op.create_control()
    }

    fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
        opcode::SendMsgZc::new(Fd(self.op.fd.as_fd().as_raw_fd()), &control.msg)
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

unsafe impl OpCode for Pipe {
    type Control = ();

    unsafe fn init(&mut self) -> Self::Control {}

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::Pipe::new(self.fds.as_mut_ptr().cast())
            .flags(libc::O_CLOEXEC as _)
            .build()
            .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        syscall!(libc::pipe2(self.fds.as_mut_ptr().cast(), libc::O_CLOEXEC)).map(|res| res as _)
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
