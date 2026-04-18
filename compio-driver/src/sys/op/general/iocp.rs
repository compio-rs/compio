use windows_sys::Win32::{
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        IO::{DeviceIoControl, OVERLAPPED},
        Pipes::ConnectNamedPipe,
    },
};

use crate::{OpCode, sys::op::*};

unsafe impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ();

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.sys_slice_mut();
        let fd = self.fd.as_fd().as_raw_fd();
        let mut transferred = 0;
        let res = unsafe {
            ReadFile(
                fd,
                slice.ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = ();

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.as_init();
        let mut transferred = 0;
        let res = unsafe {
            WriteFile(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len().try_into().unwrap_or(u32::MAX),
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();
        let mut transferred = 0;
        let slice = self.buffer.sys_slice_mut();
        let res = unsafe {
            ReadFile(
                fd,
                slice.ptr() as _,
                slice.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

unsafe impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_init();
        let mut transferred = 0;
        let res = unsafe {
            WriteFile(
                self.fd.as_fd().as_raw_fd(),
                slice.as_ptr() as _,
                slice.len().try_into().unwrap_or(u32::MAX),
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Connect to a named pipe.
pub struct ConnectNamedPipe<S> {
    pub(crate) fd: S,
}

impl<S> ConnectNamedPipe<S> {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: S) -> Self {
        Self { fd }
    }
}

unsafe impl<S: AsFd> OpCode for ConnectNamedPipe<S> {
    type Control = ();

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = unsafe { ConnectNamedPipe(self.fd.as_fd().as_raw_fd() as _, optr) };
        win32_result(res, 0)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}

/// Send a control code to a device.
#[doc(hidden)]
pub struct DeviceIoControl<S, I: IoBuf, O: IoBufMut> {
    pub(crate) fd: S,
    pub(crate) ioctl_code: u32,
    pub(crate) input_buffer: Option<I>,
    pub(crate) output_buffer: Option<O>,
}

impl<S, I: IoBuf, O: IoBufMut> DeviceIoControl<S, I, O> {
    /// Create [`DeviceIoControl`].
    pub fn new(fd: S, ioctl_code: u32, input_buffer: Option<I>, output_buffer: Option<O>) -> Self {
        Self {
            fd,
            ioctl_code,
            input_buffer,
            output_buffer,
        }
    }
}

impl<S, I: IoBuf, O: IoBufMut> IntoInner for DeviceIoControl<S, I, O> {
    type Inner = (Option<I>, Option<O>);

    fn into_inner(self) -> Self::Inner {
        (self.input_buffer, self.output_buffer)
    }
}

unsafe impl<S: AsFd, I: IoBuf, O: IoBufMut> OpCode for DeviceIoControl<S, I, O> {
    type Control = ();

    unsafe fn operate(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd.as_fd().as_raw_fd();

        let input = self
            .input_buffer
            .as_ref()
            .map_or(SysSlice::null(), |x| x.sys_slice());
        let output = self
            .output_buffer
            .as_mut()
            .map_or(SysSlice::null(), |x| x.sys_slice_mut());

        let mut transferred = 0;
        let res = unsafe {
            DeviceIoControl(
                fd,
                self.ioctl_code,
                input.ptr() as _,
                input.len() as _,
                output.ptr() as _,
                output.len() as _,
                &mut transferred,
                optr,
            )
        };
        win32_result(res, transferred)
    }

    fn cancel(&mut self, _: &mut (), optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd.as_fd().as_raw_fd(), optr)
    }
}
