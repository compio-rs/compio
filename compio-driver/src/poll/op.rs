#[cfg(aio)]
use std::ptr::NonNull;
use std::{
    ffi::CString,
    io,
    marker::PhantomPinned,
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd},
    pin::Pin,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(gnulinux))]
use libc::open;
#[cfg(gnulinux)]
use libc::open64 as open;
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "hurd")))]
use libc::{pread, preadv, pwrite, pwritev};
#[cfg(any(target_os = "linux", target_os = "android", target_os = "hurd"))]
use libc::{pread64 as pread, preadv64 as preadv, pwrite64 as pwrite, pwritev64 as pwritev};
use socket2::{SockAddr, SockAddrStorage, Socket as Socket2, socklen_t};

use super::{AsFd, Decision, OpCode, OpType, syscall};
pub use crate::unix::op::*;
use crate::{op::*, sys_slice::*};

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let this = unsafe { self.get_unchecked_mut() };
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        this.data = Some(data);
        Poll::Ready(res)
    }
}

impl<
    S,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for AsyncifyFd<S, F, D>
{
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let this = unsafe { self.get_unchecked_mut() };
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&this.fd);
        this.data = Some(data);
        Poll::Ready(res)
    }
}

impl OpCode for OpenFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(open(
            self.path.as_ptr(),
            self.flags | libc::O_CLOEXEC,
            self.mode as libc::c_int
        ))? as _))
    }
}

impl OpCode for CloseFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(libc::close(self.fd.as_fd().as_raw_fd()))? as _))
    }
}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: S,
    pub(crate) stat: libc::stat,
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        #[cfg(gnulinux)]
        {
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            static EMPTY_NAME: &[u8] = b"\0";
            syscall!(libc::statx(
                this.fd.as_fd().as_raw_fd(),
                EMPTY_NAME.as_ptr().cast(),
                libc::AT_EMPTY_PATH,
                0,
                &mut s
            ))?;
            this.stat = statx_to_stat(s);
            Poll::Ready(Ok(0))
        }
        #[cfg(not(gnulinux))]
        {
            Poll::Ready(Ok(
                syscall!(libc::fstat(this.fd.as_fd().as_raw_fd(), &mut this.stat))? as _,
            ))
        }
    }
}

impl<S> IntoInner for FileStat<S> {
    type Inner = libc::stat;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

/// Get metadata from path.
pub struct PathStat {
    pub(crate) path: CString,
    pub(crate) stat: libc::stat,
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        #[cfg(gnulinux)]
        {
            let mut flags = libc::AT_EMPTY_PATH;
            if !self.follow_symlink {
                flags |= libc::AT_SYMLINK_NOFOLLOW;
            }
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            syscall!(libc::statx(
                libc::AT_FDCWD,
                self.path.as_ptr(),
                flags,
                0,
                &mut s
            ))?;
            self.stat = statx_to_stat(s);
            Poll::Ready(Ok(0))
        }
        #[cfg(not(gnulinux))]
        {
            let f = if self.follow_symlink {
                libc::stat
            } else {
                libc::lstat
            };
            Poll::Ready(Ok(syscall!(f(self.path.as_ptr(), &mut self.stat))? as _))
        }
    }
}

impl IntoInner for PathStat {
    type Inner = libc::stat;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            let this = unsafe { self.get_unchecked_mut() };
            let slice = this.buffer.as_uninit();

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = this.offset as _;
            this.aiocb.aio_buf = slice.as_mut_ptr().cast();
            this.aiocb.aio_nbytes = slice.len();

            Ok(Decision::aio(&mut this.aiocb, libc::aio_read))
        }
        #[cfg(not(aio))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(
            &mut unsafe { self.get_unchecked_mut() }.aiocb,
        )))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let offset = self.offset;
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_uninit();
        syscall!(break pread(fd, slice.as_mut_ptr() as _, slice.len() as _, offset as _,))
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            let this = unsafe { self.get_unchecked_mut() };
            this.slices = unsafe { this.buffer.sys_slices_mut() };

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = this.offset as _;
            this.aiocb.aio_buf = this.slices.as_mut_ptr().cast();
            this.aiocb.aio_nbytes = this.slices.len();

            Ok(Decision::aio(&mut this.aiocb, libc::aio_readv))
        }
        #[cfg(not(freebsd))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(
            &mut unsafe { self.get_unchecked_mut() }.aiocb,
        )))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.sys_slices_mut() };
        syscall!(
            break preadv(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                this.offset as _,
            )
        )
    }
}

impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            let this = unsafe { self.get_unchecked_mut() };
            let slice = this.buffer.as_slice();

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = this.offset as _;
            this.aiocb.aio_buf = slice.as_ptr().cast_mut().cast();
            this.aiocb.aio_nbytes = slice.len();

            Ok(Decision::aio(&mut this.aiocb, libc::aio_write))
        }
        #[cfg(not(aio))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(
            &mut unsafe { self.get_unchecked_mut() }.aiocb,
        )))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
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

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(freebsd)]
        {
            let this = unsafe { self.get_unchecked_mut() };
            this.slices = unsafe { this.buffer.sys_slices() };

            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();
            this.aiocb.aio_offset = this.offset as _;
            this.aiocb.aio_buf = this.slices.as_ptr().cast_mut().cast();
            this.aiocb.aio_nbytes = this.slices.len();

            Ok(Decision::aio(&mut this.aiocb, libc::aio_writev))
        }
        #[cfg(not(freebsd))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(freebsd)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(
            &mut unsafe { self.get_unchecked_mut() }.aiocb,
        )))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.sys_slices() };
        syscall!(
            break pwritev(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                this.offset as _,
            )
        )
    }
}

impl<S: AsFd> OpCode for crate::op::managed::ReadManagedAt<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op) }.pre_submit()
    }

    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op) }.op_type()
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op) }.operate()
    }
}

impl<S: AsFd> OpCode for Sync<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        #[cfg(aio)]
        {
            unsafe extern "C" fn aio_fsync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_SYNC, aiocbp) }
            }
            unsafe extern "C" fn aio_fdatasync(aiocbp: *mut libc::aiocb) -> i32 {
                unsafe { libc::aio_fsync(libc::O_DSYNC, aiocbp) }
            }

            let this = unsafe { self.get_unchecked_mut() };
            this.aiocb.aio_fildes = this.fd.as_fd().as_raw_fd();

            let f = if this.datasync {
                aio_fdatasync
            } else {
                aio_fsync
            };

            Ok(Decision::aio(&mut this.aiocb, f))
        }
        #[cfg(not(aio))]
        {
            Ok(Decision::Blocking)
        }
    }

    #[cfg(aio)]
    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        Some(OpType::Aio(NonNull::from(
            &mut unsafe { self.get_unchecked_mut() }.aiocb,
        )))
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

impl OpCode for Unlink {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        if self.dir {
            syscall!(libc::rmdir(self.path.as_ptr()))?;
        } else {
            syscall!(libc::unlink(self.path.as_ptr()))?;
        }
        Poll::Ready(Ok(0))
    }
}

impl OpCode for CreateDir {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(libc::mkdir(self.path.as_ptr(), self.mode))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for Rename {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(libc::rename(self.old_path.as_ptr(), self.new_path.as_ptr()))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for Symlink {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(libc::symlink(self.source.as_ptr(), self.target.as_ptr()))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for HardLink {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(libc::link(self.source.as_ptr(), self.target.as_ptr()))?;
        Poll::Ready(Ok(0))
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
        Ok(socket.into_raw_fd())
    }
}

impl OpCode for CreateSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(unsafe { self.call()? } as _))
    }
}

impl<S: AsFd> OpCode for ShutdownSocket<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(libc::shutdown(self.fd.as_fd().as_raw_fd(), self.how()))? as _,
        ))
    }
}

impl OpCode for CloseSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::Blocking)
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(libc::close(self.fd.as_fd().as_raw_fd()))? as _))
    }
}

impl<S: AsFd> Accept<S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::c_int {
        let this = unsafe { self.get_unchecked_mut() };
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
        unsafe {
            libc::accept4(
                this.fd.as_fd().as_raw_fd(),
                &mut this.buffer as *mut _ as *mut _,
                &mut this.addr_len,
                libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            )
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
            || -> io::Result<libc::c_int> {
                let fd = syscall!(libc::accept(
                    this.fd.as_fd().as_raw_fd(),
                    &mut this.buffer as *mut _ as *mut _,
                    &mut this.addr_len,
                ))?;
                let socket = unsafe { Socket2::from_raw_fd(fd) };
                socket.set_cloexec(true)?;
                socket.set_nonblocking(true)?;
                Ok(socket.into_raw_fd())
            }()
            .unwrap_or(-1)
        }
    }
}

impl<S: AsFd> OpCode for Accept<S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(mut self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let res = syscall!(break self.as_mut().call());
        if let Poll::Ready(Ok(fd)) = res {
            unsafe {
                self.get_unchecked_mut().accepted_fd = Some(OwnedFd::from_raw_fd(fd as _));
            }
        }
        res
    }
}

impl<S: AsFd> OpCode for Connect<S> {
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
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
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

impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_uninit();
        syscall!(break libc::read(fd, slice.as_mut_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.sys_slices_mut() };
        syscall!(
            break libc::readv(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _
            )
        )
    }
}

impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
        syscall!(
            break libc::write(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len()
            )
        )
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.sys_slices() };
        syscall!(
            break libc::writev(
                this.fd.as_fd().as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _
            )
        )
    }
}

impl<S: AsFd> OpCode for crate::op::managed::RecvManaged<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op) }.pre_submit()
    }

    fn op_type(self: Pin<&mut Self>) -> Option<crate::OpType> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op) }.op_type()
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|this| &mut this.op) }.operate()
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddrStorage,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoBufMut, S> RecvFrom<T, S> {
    /// Create [`RecvFrom`].
    pub fn new(fd: S, buffer: T) -> Self {
        let addr = SockAddrStorage::zeroed();
        let addr_len = addr.size_of();
        Self {
            fd,
            buffer,
            addr,
            addr_len,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut, S: AsFd> RecvFrom<T, S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        let this = unsafe { self.get_unchecked_mut() };
        let fd = this.fd.as_fd().as_raw_fd();
        let slice = this.buffer.as_uninit();
        unsafe {
            libc::recvfrom(
                fd,
                slice.as_mut_ptr() as _,
                slice.len(),
                0,
                &mut this.addr as *mut _ as _,
                &mut this.addr_len,
            )
        }
    }
}

impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    fn pre_submit(mut self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd.as_fd().as_raw_fd();
        syscall!(self.as_mut().call(), wait_readable(fd))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
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

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<SysSlice>,
    pub(crate) addr: SockAddrStorage,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut, S> RecvFromVectored<T, S> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: S, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            addr: SockAddrStorage::zeroed(),
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> RecvFromVectored<T, S> {
    fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.sys_slices_mut() };
        self.msg.msg_name = &mut self.addr as *mut _ as _;
        self.msg.msg_namelen = std::mem::size_of_val(&self.addr) as _;
        self.msg.msg_iov = self.slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = self.slices.len() as _;
    }

    unsafe fn call(&mut self) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut self.msg, 0) }
    }
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.set_msg();
        syscall!(this.call(), wait_readable(this.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        syscall!(break this.call())
    }
}

impl<T: IoVectoredBufMut, S> IntoInner for RecvFromVectored<T, S> {
    type Inner = (T, SockAddrStorage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoBuf, S> SendTo<T, S> {
    /// Create [`SendTo`].
    pub fn new(fd: S, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf, S: AsFd> SendTo<T, S> {
    unsafe fn call(&self) -> libc::ssize_t {
        let slice = self.buffer.as_slice();
        unsafe {
            libc::sendto(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len(),
                0,
                self.addr.as_ptr().cast(),
                self.addr.len(),
            )
        }
    }
}

impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(self.call(), wait_writable(self.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
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

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf, S> {
    pub(crate) fd: S,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) slices: Vec<SysSlice>,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf, S> SendToVectored<T, S> {
    /// Create [`SendToVectored`].
    pub fn new(fd: S, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            slices: vec![],
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf, S: AsFd> SendToVectored<T, S> {
    fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.sys_slices() };
        self.msg.msg_name = &mut self.addr as *mut _ as _;
        self.msg.msg_namelen = std::mem::size_of_val(&self.addr) as _;
        self.msg.msg_iov = self.slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = self.slices.len() as _;
    }

    unsafe fn call(&self) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &self.msg, 0) }
    }
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.set_msg();
        syscall!(this.call(), wait_writable(this.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.call())
    }
}

impl<T: IoVectoredBuf, S> IntoInner for SendToVectored<T, S> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> RecvMsg<T, C, S> {
    unsafe fn call(&mut self) -> libc::ssize_t {
        unsafe { libc::recvmsg(self.fd.as_fd().as_raw_fd(), &mut self.msg, 0) }
    }
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        unsafe { this.set_msg() };
        syscall!(this.call(), wait_readable(this.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        let this = unsafe { self.get_unchecked_mut() };
        syscall!(break this.call())
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsg<T, C, S> {
    unsafe fn call(&self) -> libc::ssize_t {
        unsafe { libc::sendmsg(self.fd.as_fd().as_raw_fd(), &self.msg, 0) }
    }
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        unsafe { this.set_msg() };
        syscall!(this.call(), wait_writable(this.fd.as_fd().as_raw_fd()))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        syscall!(break self.call())
    }
}

impl<S: AsFd> OpCode for PollOnce<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_for(
            self.fd.as_fd().as_raw_fd(),
            self.interest,
        ))
    }

    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        Some(OpType::Fd(self.fd.as_fd().as_raw_fd()))
    }

    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }
}
