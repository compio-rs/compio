use std::{
    io,
    os::windows::{io::AsRawHandle, process::ExitStatusExt},
    pin::Pin,
    process,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{op::BufResultExt, syscall, AsRawFd, OpCode, OpType, Overlapped, RawFd};
use compio_io::{AsyncRead, AsyncWrite};
use compio_runtime::Runtime;
use windows_sys::Win32::{
    Foundation::ERROR_NOT_FOUND,
    Storage::FileSystem::{ReadFileEx, WriteFileEx},
    System::{
        Threading::GetExitCodeProcess,
        IO::{CancelIoEx, PostQueuedCompletionStatus, OVERLAPPED},
    },
};

use crate::{ChildStderr, ChildStdin, ChildStdout};

struct WaitProcess {
    child: process::Child,
}

impl WaitProcess {
    pub fn new(child: process::Child) -> Self {
        Self { child }
    }
}

impl OpCode for WaitProcess {
    fn op_type(&self) -> OpType {
        OpType::Event(self.child.as_raw_handle())
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let mut code = 0;
        syscall!(
            BOOL,
            GetExitCodeProcess(self.child.as_raw_handle() as _, &mut code)
        )?;
        Poll::Ready(Ok(code as _))
    }
}

pub async fn child_wait(child: process::Child) -> io::Result<process::ExitStatus> {
    let op = WaitProcess::new(child);
    let code = Runtime::current().submit(op).await.0?;
    Ok(process::ExitStatus::from_raw(code as _))
}

unsafe extern "system" fn apc_callback(
    _dwerrorcode: u32,
    dwnumberofbytestransfered: u32,
    lpoverlapped: *mut OVERLAPPED,
) {
    let optr: *mut Overlapped<()> = lpoverlapped.cast();
    if let Some(overlapped) = optr.as_ref() {
        syscall!(
            BOOL,
            PostQueuedCompletionStatus(
                overlapped.driver as _,
                dwnumberofbytestransfered,
                0,
                lpoverlapped,
            )
        )
        .ok();
    }
}

#[inline]
fn apc_cancel(fd: RawFd, optr: *mut OVERLAPPED) -> io::Result<()> {
    match syscall!(BOOL, CancelIoEx(fd as _, optr)) {
        Ok(_) => Ok(()),
        Err(e) => {
            if e.raw_os_error() == Some(ERROR_NOT_FOUND as _) {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

struct ReadApc<B: IoBufMut> {
    fd: RawFd,
    buffer: B,
}

impl<B: IoBufMut> ReadApc<B> {
    pub fn new(fd: RawFd, buffer: B) -> Self {
        Self { fd, buffer }
    }
}

impl<B: IoBufMut> OpCode for ReadApc<B> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd as _;
        let slice = self.get_unchecked_mut().buffer.as_mut_slice();
        syscall!(
            BOOL,
            ReadFileEx(
                fd,
                slice.as_mut_ptr() as _,
                slice.len() as _,
                optr,
                Some(apc_callback)
            )
        )?;
        Poll::Pending
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        apc_cancel(self.fd, optr)
    }
}

impl<B: IoBufMut> IntoInner for ReadApc<B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl AsRawFd for ChildStdout {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_handle()
    }
}

impl AsyncRead for ChildStdout {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.as_raw_fd();
        let op = ReadApc::new(fd, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_advanced()
    }
}

impl AsRawFd for ChildStderr {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_handle()
    }
}

impl AsyncRead for ChildStderr {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.as_raw_fd();
        let op = ReadApc::new(fd, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_advanced()
    }
}

struct WriteApc<B: IoBuf> {
    fd: RawFd,
    buffer: B,
}

impl<B: IoBuf> WriteApc<B> {
    pub fn new(fd: RawFd, buffer: B) -> Self {
        Self { fd, buffer }
    }
}

impl<B: IoBuf> OpCode for WriteApc<B> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
        syscall!(
            BOOL,
            WriteFileEx(
                self.fd as _,
                slice.as_ptr() as _,
                slice.len() as _,
                optr,
                Some(apc_callback),
            )
        )?;
        Poll::Pending
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        apc_cancel(self.fd, optr)
    }
}

impl<B: IoBuf> IntoInner for WriteApc<B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl AsRawFd for ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_handle()
    }
}

impl AsyncWrite for ChildStdin {
    async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
        let fd = self.as_raw_fd();
        let op = WriteApc::new(fd, buffer);
        Runtime::current().submit(op).await.into_inner()
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}
