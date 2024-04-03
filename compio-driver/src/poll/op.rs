use std::{ffi::CString, io, marker::PhantomPinned, pin::Pin, task::Poll};

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
use socket2::SockAddr;

use super::{sockaddr_storage, socklen_t, syscall, Decision, OpCode, RawFd};
use crate::op::*;
pub use crate::unix::op::*;

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
        Poll::Ready(Ok(syscall!(libc::close(self.fd))? as _))
    }
}

/// Get metadata of an opened file.
pub struct FileStat {
    pub(crate) fd: RawFd,
    pub(crate) stat: libc::stat,
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
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(mut self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        {
            let mut s: libc::statx = unsafe { std::mem::zeroed() };
            static EMPTY_NAME: &[u8] = b"\0";
            syscall!(libc::statx(
                self.fd,
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
            Poll::Ready(Ok(syscall!(libc::fstat(self.fd, &mut self.stat))? as _))
        }
    }
}

impl IntoInner for FileStat {
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

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_readable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd;
        let offset = self.offset;
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_mut_slice();
        syscall!(break pread(fd, slice.as_mut_ptr() as _, slice.len() as _, offset as _,))
    }
}

impl<T: IoVectoredBufMut> OpCode for ReadVectoredAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_readable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices_mut() };
        syscall!(
            break preadv(
                this.fd,
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                this.offset as _,
            )
        )
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(
            break pwrite(
                self.fd,
                slice.as_ptr() as _,
                slice.len() as _,
                self.offset as _,
            )
        )
    }
}

impl<T: IoVectoredBuf> OpCode for WriteVectoredAt<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices() };
        syscall!(
            break pwritev(
                this.fd,
                this.slices.as_ptr() as _,
                this.slices.len() as _,
                this.offset as _,
            )
        )
    }
}

impl OpCode for Sync {
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
                libc::fdatasync(self.fd)
            } else {
                libc::fsync(self.fd)
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
            Poll::Ready(Ok(syscall!(libc::fsync(self.fd))? as _))
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

impl OpCode for ShutdownSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(libc::shutdown(self.fd, self.how()))? as _))
    }
}

impl OpCode for CloseSocket {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::blocking_dummy())
    }

    fn on_event(self: Pin<&mut Self>, _: &Event) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(libc::close(self.fd))? as _))
    }
}

impl Accept {
    unsafe fn call(self: Pin<&mut Self>) -> libc::c_int {
        let this = self.get_unchecked_mut();
        libc::accept(
            this.fd,
            &mut this.buffer as *mut _ as *mut _,
            &mut this.addr_len,
        )
    }
}

impl OpCode for Accept {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd;
        syscall!(self.call(), wait_readable(fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl OpCode for Connect {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(
            libc::connect(self.fd, self.addr.as_ptr(), self.addr.len()),
            wait_writable(self.fd)
        )
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let mut err: libc::c_int = 0;
        let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

        syscall!(libc::getsockopt(
            self.fd,
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

impl<T: IoBufMut> OpCode for Recv<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let fd = self.fd;
        let slice = unsafe { self.get_unchecked_mut() }.buffer.as_mut_slice();
        syscall!(break libc::read(fd, slice.as_mut_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices_mut() };
        syscall!(break libc::readv(this.fd, this.slices.as_ptr() as _, this.slices.len() as _))
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let slice = self.buffer.as_slice();
        syscall!(break libc::write(self.fd, slice.as_ptr() as _, slice.len()))
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        let this = unsafe { self.get_unchecked_mut() };
        this.slices = unsafe { this.buffer.as_io_slices() };
        syscall!(break libc::writev(this.fd, this.slices.as_ptr() as _, this.slices.len() as _))
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: sockaddr_storage,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<sockaddr_storage>() as _,
            _p: PhantomPinned,
        }
    }

    unsafe fn call(self: Pin<&mut Self>) -> libc::ssize_t {
        let this = self.get_unchecked_mut();
        let fd = this.fd;
        let slice = this.buffer.as_mut_slice();
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

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let fd = self.fd;
        syscall!(self.call(), wait_readable(fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        syscall!(break self.call())
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) slices: Vec<IoSliceMut>,
    pub(crate) addr: sockaddr_storage,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut> RecvFromVectored<T> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            slices: vec![],
            addr: unsafe { std::mem::zeroed() },
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }

    fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.as_io_slices_mut() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: std::mem::size_of_val(&self.addr) as _,
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
    }

    unsafe fn call(&mut self) -> libc::ssize_t {
        libc::recvmsg(self.fd, &mut self.msg, 0)
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.set_msg();
        syscall!(this.call(), wait_readable(this.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.readable);

        let this = unsafe { self.get_unchecked_mut() };
        syscall!(break this.call())
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvFromVectored<T> {
    type Inner = (T, sockaddr_storage, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.msg.msg_namelen)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            _p: PhantomPinned,
        }
    }

    unsafe fn call(&self) -> libc::ssize_t {
        let slice = self.buffer.as_slice();
        libc::sendto(
            self.fd,
            slice.as_ptr() as _,
            slice.len(),
            0,
            self.addr.as_ptr(),
            self.addr.len(),
        )
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        syscall!(self.call(), wait_writable(self.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.call())
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
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    pub(crate) slices: Vec<IoSlice>,
    pub(crate) msg: libc::msghdr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf> SendToVectored<T> {
    /// Create [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            slices: vec![],
            msg: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }

    fn set_msg(&mut self) {
        self.slices = unsafe { self.buffer.as_io_slices() };
        self.msg = libc::msghdr {
            msg_name: &mut self.addr as *mut _ as _,
            msg_namelen: std::mem::size_of_val(&self.addr) as _,
            msg_iov: self.slices.as_mut_ptr() as _,
            msg_iovlen: self.slices.len() as _,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
    }

    unsafe fn call(&self) -> libc::ssize_t {
        libc::sendmsg(self.fd, &self.msg, 0)
    }
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision> {
        let this = unsafe { self.get_unchecked_mut() };
        this.set_msg();
        syscall!(this.call(), wait_writable(this.fd))
    }

    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>> {
        debug_assert!(event.writable);

        syscall!(break self.call())
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}
