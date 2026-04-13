#[cfg(aio)]
use std::ptr::NonNull;
use std::{
    ffi::CString,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "hurd")))]
use libc::{pread, preadv, pwrite, pwritev};
#[cfg(any(target_os = "linux", target_os = "android", target_os = "hurd"))]
use libc::{pread64 as pread, preadv64 as preadv, pwrite64 as pwrite, pwritev64 as pwritev};
use socket2::{SockAddr, SockAddrStorage, Socket as Socket2, socklen_t};

pub use self::{
    Send as SendZc, SendMsg as SendMsgZc, SendTo as SendToZc, SendToVectored as SendToVectoredZc,
    SendVectored as SendVectoredZc,
};
use super::{AsFd, Decision, OpCode, OpType, syscall};
pub use crate::sys::unix_op::*;
use crate::{op::*, sys::aio::*, sys_slice::*};

unsafe impl<D, F> OpCode for Asyncify<F, D>
where
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd);
        self.data = Some(data);
        Poll::Ready(res)
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd1, &self.fd2);
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.call(control)?;
        self.opened_fd = Some(unsafe { OwnedFd::from_raw_fd(fd as _) });
        Poll::Ready(Ok(fd))
    }
}

unsafe impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for TruncateFile<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
    pub(crate) stat: Stat,
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(gnulinux)]
        {
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            static EMPTY_NAME: &[u8] = b"\0";
            syscall!(libc::statx(
                self.fd.as_fd().as_raw_fd(),
                EMPTY_NAME.as_ptr().cast(),
                libc::AT_EMPTY_PATH,
                statx_mask(),
                &mut s
            ))?;
            self.stat = statx_to_stat(s);
            Poll::Ready(Ok(0))
        }
        #[cfg(not(gnulinux))]
        {
            Poll::Ready(Ok(
                syscall!(libc::fstat(self.fd.as_fd().as_raw_fd(), &raw mut self.stat))? as _,
            ))
        }
    }
}

impl<S> IntoInner for FileStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

/// Get metadata from path.
pub struct PathStat<S: AsFd> {
    pub(crate) dirfd: S,
    pub(crate) path: CString,
    pub(crate) stat: Stat,
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(gnulinux)]
        let res = {
            let mut flags = libc::AT_EMPTY_PATH;
            if !self.follow_symlink {
                flags |= libc::AT_SYMLINK_NOFOLLOW;
            }
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            let res = syscall!(libc::statx(
                self.dirfd.as_fd().as_raw_fd(),
                self.path.as_ptr(),
                flags,
                statx_mask(),
                &mut s
            ))?;
            self.stat = statx_to_stat(s);
            res
        };
        // Some platforms don't support `AT_EMPTY_PATH`, so we have to use `fstat` when
        // the path is empty.
        #[cfg(not(gnulinux))]
        let res = if self.path.is_empty() {
            syscall!(libc::fstat(
                self.dirfd.as_fd().as_raw_fd(),
                &raw mut self.stat
            ))?
        } else {
            syscall!(libc::fstatat(
                self.dirfd.as_fd().as_raw_fd(),
                self.path.as_ptr(),
                &raw mut self.stat,
                if !self.follow_symlink {
                    libc::AT_SYMLINK_NOFOLLOW
                } else {
                    0
                }
            ))?
        };
        Poll::Ready(Ok(res as _))
    }
}

impl<S: AsFd> IntoInner for PathStat<S> {
    type Inner = Stat;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

pub struct ReadAtControl {
    #[allow(dead_code)]
    aiocb: aiocb,
}

impl Default for ReadAtControl {
    fn default() -> Self {
        Self { aiocb: new_aiocb() }
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ReadAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        _ = ctrl;
        #[cfg(aio)]
        {
            let slice = self.buffer.sys_slice_mut();

            ctrl.aiocb.aio_fildes = self.fd.as_fd().as_raw_fd();
            ctrl.aiocb.aio_offset = self.offset as _;
            ctrl.aiocb.aio_buf = slice.ptr().cast();
            ctrl.aiocb.aio_nbytes = slice.len();
        }
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_read))
        }
        #[cfg(not(aio))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let offset = self.offset;
        let slice = self.buffer.sys_slice_mut();
        syscall!(break pread(fd, slice.ptr() as _, slice.len() as _, offset as _,))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = ReadVectoredAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_readv))
        }
        #[cfg(not(freebsd))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break preadv(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                self.offset as _,
            )
        )
    }
}

pub struct WriteAtControl {
    #[allow(dead_code)]
    aiocb: aiocb,
}

impl Default for WriteAtControl {
    fn default() -> Self {
        Self { aiocb: new_aiocb() }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = WriteAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        _ = ctrl;
        #[cfg(aio)]
        {
            let slice = self.buffer.as_init();

            ctrl.aiocb.aio_fildes = self.fd.as_fd().as_raw_fd();
            ctrl.aiocb.aio_offset = self.offset as _;
            ctrl.aiocb.aio_buf = slice.as_ptr() as _;
            ctrl.aiocb.aio_nbytes = slice.len();
        }
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_write))
        }
        #[cfg(not(aio))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        syscall!(
            break pwrite(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _,
            )
        )
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = WriteVectoredAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            Ok(Decision::aio(&mut ctrl.aiocb, libc::aio_writev))
        }
        #[cfg(not(freebsd))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break pwritev(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _,
                self.offset as _,
            )
        )
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::ReadManagedAt<S> {
    type Control = ReadAtControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.buffer.sys_slice_mut();
        syscall!(break libc::read(fd, slice.ptr() as _, slice.len()))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = ReadVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break libc::readv(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _
            )
        )
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        syscall!(
            break libc::write(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len()
            )
        )
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = WriteVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(
            break libc::writev(
                self.fd.as_fd().as_raw_fd(),
                control.slices.as_ptr() as _,
                control.slices.len() as _
            )
        )
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::ReadManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}

pub struct SyncControl {
    #[allow(dead_code)]
    aiocb: aiocb,
}

impl Default for SyncControl {
    fn default() -> Self {
        Self { aiocb: new_aiocb() }
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = SyncControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        _ = ctrl;
        #[cfg(aio)]
        {
            ctrl.aiocb.aio_fildes = self.fd.as_fd().as_raw_fd();
        }
    }

    fn pre_submit(&mut self, ctrl: &mut Self::Control) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            unsafe extern "C" fn aio_fsync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_SYNC, aiocbp) }
            }
            unsafe extern "C" fn aio_fdatasync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_DSYNC, aiocbp) }
            }

            let f = if self.datasync {
                aio_fdatasync
            } else {
                aio_fsync
            };

            Ok(Decision::aio(&mut ctrl.aiocb, f))
        }
        #[cfg(not(aio))]
        {
            _ = ctrl;
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(&mut control.aiocb)))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(datasync)]
        {
            Poll::Ready(Ok(syscall!(if self.datasync {
                libc::fdatasync(self.fd.as_fd().as_raw_fd())
            } else {
                libc::fsync(self.fd.as_fd().as_raw_fd())
            })? as _))
        }
        #[cfg(not(datasync))]
        {
            Poll::Ready(Ok(syscall!(libc::fsync(self.fd.as_fd().as_raw_fd()))? as _))
        }
    }
}

unsafe impl<S: AsFd> OpCode for Unlink<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for CreateDir<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for Symlink<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

impl CreateSocket {
    unsafe fn call(&mut self, _control: &mut ()) -> io::Result<libc::c_int> {
        #[allow(unused_mut)]
        let mut ty: i32 = self.socket_type;
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "cygwin",
        ))]
        {
            ty |= libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK;
        }
        let fd = syscall!(libc::socket(self.domain, ty, self.protocol))?;
        let socket = unsafe { Socket2::from_raw_fd(fd) };
        #[cfg(not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "espidf",
            target_os = "vita",
            target_os = "cygwin",
        )))]
        socket.set_cloexec(true)?;
        #[cfg(target_vendor = "apple")]
        socket.set_nosigpipe(true)?;
        #[cfg(not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "cygwin",
        )))]
        socket.set_nonblocking(true)?;
        self.opened_fd = Some(socket);
        Ok(fd)
    }
}

unsafe impl OpCode for CreateSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(unsafe { self.call(control)? } as _))
    }
}

unsafe impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(
            syscall!(libc::bind(
                self.fd.as_fd().as_raw_fd(),
                self.addr.as_ptr().cast(),
                self.addr.len() as socklen_t
            ))
            .map(|res| res as _),
        )
    }
}

unsafe impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(
            syscall!(libc::listen(self.fd.as_fd().as_raw_fd(), self.backlog)).map(|res| res as _),
        )
    }
}

unsafe impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

unsafe impl OpCode for CloseSocket {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call(control))
    }
}

impl<S: AsFd> Accept<S> {
    // If the first call succeeds, there won't be another call.
    unsafe fn call(&mut self, _control: &mut ()) -> libc::c_int {
        || -> io::Result<libc::c_int> {
            #[cfg(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "linux",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "cygwin",
            ))]
            {
                let fd = syscall!(libc::accept4(
                    self.fd.as_fd().as_raw_fd(),
                    &raw mut self.buffer as *mut _,
                    &raw mut self.addr_len,
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                self.accepted_fd = Some(socket);
                Ok(fd)
            }
            #[cfg(not(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "fuchsia",
                target_os = "illumos",
                target_os = "linux",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "cygwin",
            )))]
            {
                let fd = syscall!(libc::accept(
                    self.fd.as_fd().as_raw_fd(),
                    &raw mut self.buffer as *mut _,
                    &raw mut self.addr_len,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                socket.set_cloexec(true)?;
                socket.set_nonblocking(true)?;
                self.accepted_fd = Some(socket);
                Ok(fd)
            }
        }()
        .unwrap_or(-1)
    }
}

unsafe impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

/// Accept multiple connections.
pub struct AcceptMulti<S> {
    pub(crate) op: Accept<S>,
}

impl<S> AcceptMulti<S> {
    /// Create [`AcceptMulti`].
    pub fn new(fd: S) -> Self {
        Self {
            op: Accept::new(fd),
        }
    }
}

unsafe impl<S: AsFd> OpCode for AcceptMulti<S> {
    type Control = <Accept<S> as OpCode>::Control;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
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

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        syscall!(
            libc::connect(
                self.fd.as_fd().as_raw_fd(),
                self.addr.as_ptr().cast(),
                self.addr.len()
            ),
            wait_writable(self.fd.as_fd().as_raw_fd())
        )
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(libc::getsockopt(
            self.fd.as_fd().as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut err as *mut _ as *mut _,
            &mut err_len
        ))?;

        let res = if err == 0 {
            Ok(0)
        } else {
            Err(io::Error::from_raw_os_error(err))
        };
        Poll::Ready(res)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let flags = self.flags;
        let slice = self.buffer.sys_slice_mut();
        syscall!(break libc::recv(fd, slice.ptr() as _, slice.len(), flags))
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        syscall!(
            break libc::send(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len(),
                self.flags,
            )
        )
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::RecvManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<crate::OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvVectored<T, S> {
    unsafe fn call(&mut self, control: &mut RecvVectoredControl) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = RecvVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendVectored<T, S> {
    unsafe fn call(&self, control: &mut SendVectoredControl) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = SendVectoredControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_writable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    pub(crate) flags: i32,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        let addr = SockAddrStorage::zeroed();
        let addr_len = addr.size_of();

        Self {
            fd,
            buffer,
            addr,
            addr_len,
            flags,
        }
    }
}

impl<T: IoBufMut, S: AsFd> RecvFrom<T, S> {
    unsafe fn call(&mut self, _control: &mut ()) -> libc::ssize_t {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.buffer.sys_slice_mut();

        unsafe {
            libc::recvfrom(
                fd,
                slice.ptr() as _,
                slice.len(),
                self.flags,
                &raw mut self.addr as _,
                &raw mut self.addr_len,
            )
        }
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = (self.addr_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.addr_len) });
        (self.buffer, addr)
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::RecvFromManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::RecvFromMulti<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }

    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        result: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        if let Ok(result) = result {
            self.len = *result;
        }
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) name_len: socklen_t,
    pub(crate) flags: i32,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr: SockAddrStorage::zeroed(),
            name_len: 0,
            flags,
        }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvFromVectored<T, S> {
    unsafe fn call(&mut self, control: &mut SendMsgControl) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices_mut().into();
        ctrl.msg.msg_name = &raw mut self.addr as _;
        ctrl.msg.msg_namelen = self.addr.size_of() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_mut_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
        self.name_len = control.msg.msg_namelen;
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, Option<SockAddr>);

    fn into_inner(self) -> Self::Inner {
        let addr = (self.name_len > 0).then(|| unsafe { SockAddr::new(self.addr, self.name_len) });
        (self.buffer, addr)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    flags: i32,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
        }
    }
}

impl<T: IoBuf, S: AsFd> SendTo<T, S> {
    unsafe fn call(&self) -> libc::ssize_t {
        let slice = self.buffer.as_init();
        unsafe {
            libc::sendto(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len(),
                self.flags,
                self.addr.as_ptr().cast(),
                self.addr.len(),
            )
        }
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        syscall!(self.call(), wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call())
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
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) flags: i32,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
        }
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectored<T, S> {
    unsafe fn call(&mut self, control: &mut SendMsgControl) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        ctrl.slices = self.buffer.sys_slices().into();
        ctrl.msg.msg_name = self.addr.as_ptr() as _;
        ctrl.msg.msg_namelen = self.addr.len() as _;
        ctrl.msg.msg_iov = ctrl.slices.as_ptr() as _;
        ctrl.msg.msg_iovlen = ctrl.slices.len() as _;
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_writable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> RecvMsg<T, C, S> {
    unsafe fn call(&mut self, control: &mut RecvMsgControl) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_readable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
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

unsafe impl<C: IoBufMut, S: AsFd> OpCode for crate::op::managed::RecvMsgManaged<C, S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, result, extra) }
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::RecvMsgMulti<S> {
    type Control = RecvMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        unsafe { self.op.init(ctrl) }
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        self.op.pre_submit(control)
    }

    fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
        self.op.op_type(control)
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        self.op.operate(control)
    }

    unsafe fn set_result(
        &mut self,
        control: &mut Self::Control,
        result: &io::Result<usize>,
        extra: &crate::Extra,
    ) {
        unsafe { self.op.set_result(control, result, extra) };
        if let Ok(result) = result {
            self.len = *result;
        }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsg<T, C, S> {
    unsafe fn call(&mut self, control: &mut SendMsgControl) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &control.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = SendMsgControl;

    unsafe fn init(&mut self, ctrl: &mut Self::Control) {
        self.create_control(ctrl)
    }

    fn pre_submit(&mut self, control: &mut Self::Control) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.call(control), wait_writable(fd))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, control: &mut Self::Control) -> Poll<io::Result<usize>> {
        syscall!(break self.call(control))
    }
}

unsafe impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::wait_for(
            self.fd.as_fd().as_raw_fd(),
            self.interest,
        ))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }
}

unsafe impl OpCode for Pipe {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(&mut self, _: &mut Self::Control) -> Poll<io::Result<usize>> {
        #[cfg(any(freebsd, solarish, linux_all))]
        {
            Poll::Ready(
                syscall!(libc::pipe2(
                    self.fds.as_mut_ptr().cast(),
                    libc::O_CLOEXEC | libc::O_NONBLOCK
                ))
                .map(|res| res as _),
            )
        }
        #[cfg(not(any(freebsd, solarish, linux_all)))]
        {
            use nix::fcntl::{F_GETFD, F_GETFL, F_SETFD, F_SETFL, FdFlag, OFlag, fcntl};

            syscall!(libc::pipe(self.fds.as_mut_ptr().cast()))?;
            let Some(f1) = self.fds[0].as_ref() else {
                unreachable!("pipe() succeeded but returned invalid fd")
            };
            let Some(f2) = self.fds[1].as_ref() else {
                unreachable!("pipe() succeeded but returned invalid fd")
            };

            fn set_cloexec(fd: &OwnedFd) -> nix::Result<()> {
                let flag = FdFlag::from_bits_retain(fcntl(fd, F_GETFD)?);
                fcntl(fd, F_SETFD(flag | FdFlag::FD_CLOEXEC))?;
                Ok(())
            }

            fn set_nonblock(fd: &OwnedFd) -> nix::Result<()> {
                let flag = OFlag::from_bits_retain(fcntl(fd, F_GETFL)?);
                fcntl(fd, F_SETFL(flag | OFlag::O_NONBLOCK))?;
                Ok(())
            }

            set_cloexec(f1)?;
            set_cloexec(f2)?;
            set_nonblock(f1)?;
            set_nonblock(f2)?;

            Poll::Ready(Ok(0))
        }
    }
}

#[cfg(linux_all)]
unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn pre_submit(&mut self, _control: &mut Self::Control) -> io::Result<Decision> {
        use super::WaitArg;

        Ok(Decision::wait_for_many([
            WaitArg::readable(self.fd_in.as_fd().as_raw_fd()),
            WaitArg::writable(self.fd_out.as_fd().as_raw_fd()),
        ]))
    }

    fn op_type(&mut self, _control: &mut Self::Control) -> Option<OpType> {
        Some(OpType::multi_fd([
            self.fd_in.as_fd().as_raw_fd(),
            self.fd_out.as_fd().as_raw_fd(),
        ]))
    }

    fn operate(&mut self, _control: &mut Self::Control) -> Poll<io::Result<usize>> {
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
        syscall!(
            break libc::splice(
                self.fd_in.as_fd().as_raw_fd(),
                offset_in_ptr,
                self.fd_out.as_fd().as_raw_fd(),
                offset_out_ptr,
                self.len,
                self.flags | libc::SPLICE_F_NONBLOCK,
            )
        )
    }
}
