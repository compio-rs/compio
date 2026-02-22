#[cfg(aio)]
use std::ptr::NonNull;
use std::{
    ffi::CString,
    io,
    marker::PhantomPinned,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    pin::Pin,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "hurd")))]
use libc::{pread, preadv, pwrite, pwritev};
#[cfg(any(target_os = "linux", target_os = "android", target_os = "hurd"))]
use libc::{pread64 as pread, preadv64 as preadv, pwrite64 as pwrite, pwritev64 as pwritev};
use pin_project_lite::pin_project;
use socket2::{SockAddr, SockAddrStorage, Socket as Socket2, socklen_t};

use super::{AsFd, Decision, OpCode, OpType, syscall};
pub use crate::sys::unix_op::*;
use crate::{op::*, sys_slice::*};

unsafe impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        *this.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<
    S,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd<S, F, D>
{
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(this.fd);
        *this.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<
    S1,
    S2,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd2<S1, S2, F, D>
{
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(this.fd1, this.fd2);
        *this.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S: AsFd> OpCode for OpenFile<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let fd = self.as_mut().call()?;
        *self.project().opened_fd = Some(unsafe { OwnedFd::from_raw_fd(fd as _) });
        Poll::Ready(Ok(fd))
    }
}

unsafe impl OpCode for CloseFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for TruncateFile<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

pin_project! {
    /// Get metadata of an opened file.
    pub struct FileStat<S> {
        pub(crate) fd: S,
        pub(crate) stat: Stat,
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

unsafe impl<S: AsFd> OpCode for FileStat<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        #[cfg(gnulinux)]
        {
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            static EMPTY_NAME: &[u8] = b"\0";
            syscall!(libc::statx(
                this.fd.as_fd().as_raw_fd(),
                EMPTY_NAME.as_ptr().cast(),
                libc::AT_EMPTY_PATH,
                statx_mask(),
                &mut s
            ))?;
            *this.stat = statx_to_stat(s);
            Poll::Ready(Ok(0))
        }
        #[cfg(not(gnulinux))]
        {
            Poll::Ready(Ok(
                syscall!(libc::fstat(this.fd.as_fd().as_raw_fd(), this.stat))? as _,
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

pin_project! {
    /// Get metadata from path.
    pub struct PathStat<S: AsFd> {
        pub(crate) dirfd: S,
        pub(crate) path: CString,
        pub(crate) stat: Stat,
        pub(crate) follow_symlink: bool,
    }
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        #[cfg(gnulinux)]
        let res = {
            let mut flags = libc::AT_EMPTY_PATH;
            if !*this.follow_symlink {
                flags |= libc::AT_SYMLINK_NOFOLLOW;
            }
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            let res = syscall!(libc::statx(
                this.dirfd.as_fd().as_raw_fd(),
                this.path.as_ptr(),
                flags,
                statx_mask(),
                &mut s
            ))?;
            *this.stat = statx_to_stat(s);
            res
        };
        // Some platforms don't support `AT_EMPTY_PATH`, so we have to use `fstat` when
        // the path is empty.
        #[cfg(not(gnulinux))]
        let res = if this.path.is_empty() {
            syscall!(libc::fstat(this.dirfd.as_fd().as_raw_fd(), this.stat))?
        } else {
            syscall!(libc::fstatat(
                this.dirfd.as_fd().as_raw_fd(),
                this.path.as_ptr(),
                this.stat,
                if !*this.follow_symlink {
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

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            let this = self.project();
            let slice = this.buffer.sys_slice_mut();

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = *this.offset as _;
            this.aiocb.aio_buf = slice.ptr().cast();
            this.aiocb.aio_nbytes = slice.len();

            Ok(Decision::aio(this.aiocb, libc::aio_read))
        }
        #[cfg(not(aio))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(self.project().aiocb)))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let offset = self.offset;
        let slice = self.project().buffer.sys_slice_mut();
        syscall!(break pread(fd, slice.ptr() as _, slice.len() as _, offset as _,))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            let this = self.project();
            *this.slices = this.buffer.sys_slices_mut();

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = *this.offset as _;
            this.aiocb.aio_buf = this.slices.as_mut_ptr().cast();
            this.aiocb.aio_nbytes = this.slices.len();

            Ok(Decision::aio(this.aiocb, libc::aio_readv))
        }
        #[cfg(not(freebsd))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(self.project().aiocb)))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        *this.slices = this.buffer.sys_slices_mut();
        syscall!(
            break preadv(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                *this.offset as _,
            )
        )
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            let this = self.project();
            let slice = this.buffer.as_ref().sys_slice();

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = *this.offset as _;
            this.aiocb.aio_buf = slice.ptr().cast();
            this.aiocb.aio_nbytes = slice.len();

            Ok(Decision::aio(this.aiocb, libc::aio_write))
        }
        #[cfg(not(aio))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(self.project().aiocb)))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            let this = self.project();
            *this.slices = this.buffer.as_ref().sys_slices();

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = *this.offset as _;
            this.aiocb.aio_buf = this.slices.as_ptr().cast_mut().cast();
            this.aiocb.aio_nbytes = this.slices.len();

            Ok(Decision::aio(this.aiocb, libc::aio_writev))
        }
        #[cfg(not(freebsd))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(self.project().aiocb)))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        *this.slices = this.buffer.as_ref().sys_slices();
        syscall!(
            break pwritev(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                *this.offset as _,
            )
        )
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::ReadManagedAt<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        self.project().op.pre_submit()
    }

    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        self.project().op.op_type()
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        self.project().op.operate()
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = self.project().buffer.sys_slice_mut();
        syscall!(break libc::read(fd, slice.ptr() as _, slice.len()))
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        *this.slices = this.buffer.sys_slices_mut();
        syscall!(
            break libc::readv(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _
            )
        )
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = self.project();
        *this.slices = this.buffer.as_ref().sys_slices();
        syscall!(
            break libc::writev(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _
            )
        )
    }
}

unsafe impl<S: AsFd> OpCode for crate::op::managed::ReadManaged<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        self.project().op.pre_submit()
    }

    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        self.project().op.op_type()
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        self.project().op.operate()
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            unsafe extern "C" fn aio_fsync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_SYNC, aiocbp) }
            }
            unsafe extern "C" fn aio_fdatasync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_DSYNC, aiocbp) }
            }

            let this = self.project();
            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();

            let f = if *this.datasync {
                aio_fdatasync
            } else {
                aio_fsync
            };

            Ok(Decision::aio(this.aiocb, f))
        }
        #[cfg(not(aio))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(self.project().aiocb)))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for CreateDir<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for Rename<S1, S2> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S: AsFd> OpCode for Symlink<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl<S1: AsFd, S2: AsFd> OpCode for HardLink<S1, S2> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

impl CreateSocket {
    unsafe fn call(self: Pin<&mut Self>) -> io::Result<libc::c_int> {
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
        #[cfg(any(
            target_os = "ios",
            target_os = "macos",
            target_os = "tvos",
            target_os = "watchos",
        ))]
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
        *self.project().opened_fd = Some(socket);
        Ok(fd)
    }
}

unsafe impl OpCode for CreateSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(unsafe { self.call()? } as _))
    }
}

unsafe impl<S: AsFd> OpCode for ShutdownSocket<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

unsafe impl OpCode for CloseSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(self.call())
    }
}

impl<S: AsFd> Accept<S> {
    // If the first call succeeds, there won't be another call.
    unsafe fn call(self: Pin<&mut Self>) -> libc::c_int {
        let this = self.project();
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
                    this.fd.as_fd().as_raw_fd(),
                    this.buffer as *mut _ as *mut _,
                    this.addr_len,
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                *this.accepted_fd = Some(socket);
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
                    this.fd.as_fd().as_raw_fd(),
                    this.buffer as *mut _ as *mut _,
                    this.addr_len,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                socket.set_cloexec(true)?;
                socket.set_nonblocking(true)?;
                *this.accepted_fd = Some(socket);
                Ok(fd)
            }
        }()
        .unwrap_or(-1)
    }
}

unsafe impl<S: AsFd> OpCode for Accept<S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

unsafe impl<S: AsFd> OpCode for Connect<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            libc::connect(
                self.fd.as_fd().as_raw_fd(),
                self.addr.as_ptr().cast(),
                self.addr.len()
            ),
            wait_writable(self.fd.as_fd().as_raw_fd())
        )
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let flags = self.flags;
        let slice = self.project().buffer.sys_slice_mut();
        syscall!(break libc::recv(fd, slice.ptr() as _, slice.len(), flags))
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        self.project().op.pre_submit()
    }

    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        self.project().op.op_type()
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        self.project().op.operate()
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvVectored<T, S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        let this = self.project();
        unsafe { libc::recvmsg(this.fd.as_fd().as_raw_fd(), this.msg, *this.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.as_mut().set_msg();
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendVectored<T, S> {
    unsafe fn call(&self) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &self.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.as_mut().set_msg();
        let fd = self.as_mut().project().fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_writable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.call())
    }
}

pin_project! {
    /// Receive data and source address.
    pub struct RecvFrom<T: IoBufMut, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) addr: SockAddrStorage,
        pub(crate) addr_len: socklen_t,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
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
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut, S: AsFd> RecvFrom<T, S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        let fd = self.fd.as_fd().as_raw_fd();
        let this = self.project();
        let slice = this.buffer.sys_slice_mut();
        unsafe {
            libc::recvfrom(
                fd,
                slice.ptr() as _,
                slice.len(),
                *this.flags,
                this.addr as *mut _ as _,
                this.addr_len,
            )
        }
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

impl<T: IoBufMut, S> IntoInner for RecvFrom<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

pin_project! {
    /// Receive data and source address into vectored buffer.
    pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) addr: SockAddrStorage,
        pub(crate) msg: libc::msghdr,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            addr: SockAddrStorage::zeroed(),
            msg: unsafe { std::mem::zeroed() },
            flags,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvFromVectored<T, S> {
    fn set_msg(self: Pin<&mut Self>) {
        let this = self.project();
        *this.slices = this.buffer.sys_slices_mut();
        this.msg.msg_name = this.addr as *mut _ as _;
        this.msg.msg_namelen = this.addr.size_of() as _;
        this.msg.msg_iov = this.slices.as_mut_ptr() as _;
        this.msg.msg_iovlen = this.slices.len() as _;
    }

    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        let this = self.project();
        unsafe { libc::recvmsg(this.fd.as_fd().as_raw_fd(), this.msg, *this.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.as_mut().set_msg();
        let fd = self.as_mut().project().fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

pin_project! {
    /// Send data to specified address.
    pub struct SendTo<T: IoBuf, S> {
        pub(crate) fd: S,
        pub(crate) buffer: T,
        pub(crate) addr: SockAddr,
        flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            flags,
            _p: PhantomPinned,
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(self.call(), wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.call())
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
        pub(crate) fd: S,
        #[pin]
        pub(crate) buffer: T,
        pub(crate) addr: SockAddr,
        pub(crate) slices: Vec<SysSlice>,
        pub(crate) msg: libc::msghdr,
        pub(crate) flags: i32,
        _p: PhantomPinned,
    }
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr, flags: i32) -> Self {
        Self {
            fd,
            buffer,
            addr,
            slices: vec![],
            msg: unsafe { std::mem::zeroed() },
            flags,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectored<T, S> {
    fn set_msg(self: Pin<&mut Self>) {
        let this = self.project();
        *this.slices = this.buffer.as_ref().sys_slices();
        this.msg.msg_name = this.addr as *mut _ as _;
        this.msg.msg_namelen = this.addr.len() as _;
        this.msg.msg_iov = this.slices.as_mut_ptr() as _;
        this.msg.msg_iovlen = this.slices.len() as _;
    }

    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &self.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.as_mut().set_msg();
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_writable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> RecvMsg<T, C, S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        let this = self.project();
        unsafe { libc::recvmsg(this.fd.as_fd().as_raw_fd(), this.msg, *this.flags) }
    }
}

unsafe impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.as_mut().set_msg();
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsg<T, C, S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &self.msg, self.flags) }
    }
}

unsafe impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        self.as_mut().set_msg();
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_writable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.as_mut().call())
    }
}

unsafe impl<S: AsFd> OpCode for PollOnce<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_for(
            self.fd.as_fd().as_raw_fd(),
            self.interest,
        ))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }
}

#[cfg(linux_all)]
unsafe impl<S1: AsFd, S2: AsFd> OpCode for Splice<S1, S2> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        use super::WaitArg;

        Ok(Decision::wait_for_many([
            WaitArg::readable(self.fd_in.as_fd().as_raw_fd()),
            WaitArg::writable(self.fd_out.as_fd().as_raw_fd()),
        ]))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::multi_fd([
            self.fd_in.as_fd().as_raw_fd(),
            self.fd_out.as_fd().as_raw_fd(),
        ]))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
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
