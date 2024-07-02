use std::{ffi::CString, io, pin::Pin, task::Poll};

use compio_buf::{
    BufResult, IntoInner, IoBuf, IoBufMut, IoSlice, IoSliceMut, IoVectoredBuf, IoVectoredBufMut,
};
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
use libc::open;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use libc::open64 as open;
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "hurd")))]
use libc::{pread, preadv, pwrite, pwritev};
#[cfg(any(target_os = "linux", target_os = "android", target_os = "hurd"))]
use libc::{pread64 as pread, preadv64 as preadv, pwrite64 as pwrite, pwritev64 as pwritev};
use polling::Event;

use super::{syscall, AsRawFd, Decision, OpCode};
pub use crate::unix::op::*;
use crate::{op::*, SharedFd};

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + std::marker::Sync + 'static,
> OpCode for Asyncify<F, D>
{
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        // Safety: self won't be moved
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

impl OpCode for OpenFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(open(
            self.path.as_ptr(),
            self.flags,
            self.mode as libc::c_int
        ))? as _))
    }
}

impl OpCode for CloseFile {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(libc::close(self.fd.as_raw_fd()))? as _))
    }
}

/// Get metadata of an opened file.
pub struct FileStat<S> {
    pub(crate) fd: SharedFd<S>,
    pub(crate) stat: libc::stat,
}

impl<S> FileStat<S> {
    /// Create [`FileStat`].
    pub fn new(fd: SharedFd<S>) -> Self {
        Self {
            fd,
            stat: unsafe { std::mem::zeroed() },
        }
    }
}

impl<S: AsRawFd> OpCode for FileStat<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(mut self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        {
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            static EMPTY_NAME: &[u8] = b"\0";
            syscall!(libc::statx(
                self.fd.as_raw_fd(),
                EMPTY_NAME.as_ptr().cast(),
                libc::AT_EMPTY_PATH,
                0,
                &mut s
            ))?;
            self.stat = statx_to_stat(s);
            Poll::Ready(Ok(0))
        }
        #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
        {
            Poll::Ready(Ok(
                syscall!(libc::fstat(self.fd.as_raw_fd(), &mut self.stat))? as _,
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
        Ok(Decision::blocking_dummy())
    }

    fn on_event(mut self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
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
        #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
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

impl<T: IoBufMut, S: AsRawFd> OpCode for ReadAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_readable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd.as_raw_fd();
        let offset = self.offset;
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_mut_slice();
        syscall!(break pread(fd, slice.as_mut_ptr() as _, slice.len() as _, offset as _,))
    }
}

impl<T: IoVectoredBufMut, S: AsRawFd> OpCode for ReadVectoredAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_readable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices_mut() };
        syscall!(
            break preadv(
                this.fd.as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                this.offset as _,
            )
        )
    }
}

impl<T: IoBuf, S: AsRawFd> OpCode for WriteAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_writable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(
            break pwrite(
                self.fd.as_raw_fd(),
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _,
            )
        )
    }
}

impl<T: IoVectoredBuf, S: AsRawFd> OpCode for WriteVectoredAt<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_writable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices() };
        syscall!(
            break pwritev(
                this.fd.as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                this.offset as _,
            )
        )
    }
}

impl<S: AsRawFd> OpCode for Sync<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        #[cfg(any(
            target_os = "android",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd"
        ))]
        {
            Poll::Ready(Ok(syscall!(if self.datasync {
                libc::fdatasync(self.fd.as_raw_fd())
            } else {
                libc::fsync(self.fd.as_raw_fd())
            })? as _))
        }
        #[cfg(not(any(
            target_os = "android",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd"
        )))]
        {
            Poll::Ready(Ok(syscall!(libc::fsync(self.fd.as_raw_fd()))? as _))
        }
    }
}

impl OpCode for Unlink {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
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
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        syscall!(libc::mkdir(self.path.as_ptr(), self.mode))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for Rename {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        syscall!(libc::rename(self.old_path.as_ptr(), self.new_path.as_ptr()))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for Symlink {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        syscall!(libc::symlink(self.source.as_ptr(), self.target.as_ptr()))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for HardLink {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        syscall!(libc::link(self.source.as_ptr(), self.target.as_ptr()))?;
        Poll::Ready(Ok(0))
    }
}

impl OpCode for CreateSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(libc::socket(self.domain, self.socket_type, self.protocol))? as _,
        ))
    }
}

impl<S: AsRawFd> OpCode for ShutdownSocket<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(libc::shutdown(self.fd.as_raw_fd(), self.how()))? as _,
        ))
    }
}

impl OpCode for CloseSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(libc::close(self.fd.as_raw_fd()))? as _))
    }
}

impl<S: AsRawFd> Accept<S> {
    unsafe fn call(self: Pin<&mut Self>) -> libc::c_int {
        let this = self.get_unchecked_mut();
        libc::accept(
            this.fd.as_raw_fd(),
            &mut this.buffer as *mut _ as *mut _,
            &mut this.addr_len,
        )
    }
}

impl<S: AsRawFd> OpCode for Accept<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd.as_raw_fd();
        syscall!(self.call(), wait_readable(fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl<S: AsRawFd> OpCode for Connect<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            libc::connect(self.fd.as_raw_fd(), self.addr.as_ptr(), self.addr.len()),
            wait_writable(self.fd.as_raw_fd())
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(libc::getsockopt(
            self.fd.as_raw_fd(),
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

impl<T: IoBufMut, S: AsRawFd> OpCode for Recv<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd.as_raw_fd();
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_mut_slice();
        syscall!(break libc::read(fd, slice.as_mut_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBufMut, S: AsRawFd> OpCode for RecvVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices_mut() };
        syscall!(
            break libc::readv(
                this.fd.as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _
            )
        )
    }
}

impl<T: IoBuf, S: AsRawFd> OpCode for Send<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(break libc::write(self.fd.as_raw_fd(), slice.as_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBuf, S: AsRawFd> OpCode for SendVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd.as_raw_fd()))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices() };
        syscall!(
            break libc::writev(
                this.fd.as_raw_fd(),
                this.slices.as_ptr() as _,
                this.slices.len() as _
            )
        )
    }
}

impl<S> RecvFromHeader<S> {
    fn set_msg(&mut self, slices: &mut [IoSliceMut]) {
        self.msg.msg_name = &mut self.addr as *mut _ as _;
        self.msg.msg_namelen = std::mem::size_of_val(&self.addr) as _;
        self.msg.msg_iov = slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
    }
}

impl<S: AsRawFd> RecvFromHeader<S> {
    unsafe fn call(&mut self) -> libc::ssize_t {
        libc::recvmsg(self.fd.as_raw_fd(), &mut self.msg, 0)
    }
}

impl<T: IoBufMut, S: AsRawFd> OpCode for RecvFrom<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices[0] = unsafe { this.buffer.as_io_slice_mut() };
        this.header.set_msg(&mut this.slices);
        syscall!(
            this.header.call(),
            wait_readable(this.header.fd.as_raw_fd())
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        syscall!(break this.header.call())
    }
}

impl<T: IoVectoredBufMut, S: AsRawFd> OpCode for RecvFromVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices_mut() };
        this.header.set_msg(&mut this.slices);
        syscall!(
            this.header.call(),
            wait_readable(this.header.fd.as_raw_fd())
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        syscall!(break this.header.call())
    }
}

impl<S> SendToHeader<S> {
    fn set_msg(&mut self, slices: &mut [IoSlice]) {
        self.msg.msg_name = self.addr.as_ptr() as _;
        self.msg.msg_namelen = self.addr.len();
        self.msg.msg_iov = slices.as_mut_ptr() as _;
        self.msg.msg_iovlen = slices.len() as _;
    }
}

impl<S: AsRawFd> SendToHeader<S> {
    unsafe fn call(&self) -> libc::ssize_t {
        libc::sendmsg(self.fd.as_raw_fd(), &self.msg, 0)
    }
}

impl<T: IoBuf, S: AsRawFd> OpCode for SendTo<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices[0] = unsafe { this.buffer.as_io_slice() };
        this.header.set_msg(&mut this.slices);
        syscall!(
            this.header.call(),
            wait_writable(this.header.fd.as_raw_fd())
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.header.call())
    }
}

impl<T: IoVectoredBuf, S: AsRawFd> OpCode for SendToVectored<T, S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices() };
        this.header.set_msg(&mut this.slices);
        syscall!(
            this.header.call(),
            wait_writable(this.header.fd.as_raw_fd())
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.header.call())
    }
}

impl<S: AsRawFd> OpCode for PollOnce<S> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_for(self.fd.as_raw_fd(), self.interest))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        match self.interest {
            Interest::Readable => debug_assert!(event.readable),
            Interest::Writable => debug_assert!(event.writable),
        }

        Poll::Ready(Ok(0))
    }
}
