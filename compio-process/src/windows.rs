use std::{
    io,
    os::windows::{io::AsRawHandle, process::ExitStatusExt},
    process,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    BufferRef, OpCode, OpType, ResultTakeBuffer, ToSharedFd,
    op::{BufResultExt, Read, ReadManaged, Write},
    syscall,
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};
use compio_runtime::Runtime;
use windows_sys::Win32::System::{IO::OVERLAPPED, Threading::GetExitCodeProcess};

use crate::{ChildStderr, ChildStdin, ChildStdout};

struct WaitProcess {
    child: process::Child,
}

impl WaitProcess {
    pub fn new(child: process::Child) -> Self {
        Self { child }
    }
}

unsafe impl OpCode for WaitProcess {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Event(self.child.as_raw_handle() as _)
    }

    unsafe fn operate(
        &mut self,
        _: &mut Self::Control,
        _optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
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
    let code = compio_runtime::submit(op).await.0?;
    Ok(process::ExitStatus::from_raw(code as _))
}

impl AsyncRead for ChildStdout {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Read::new(fd, buffer);
        let res = compio_runtime::submit(op).await.into_inner();
        unsafe { res.map_advanced() }
    }
}

impl AsyncReadManaged for ChildStdout {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        let fd = self.to_shared_fd();
        let res = Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = ReadManaged::new(fd, &buffer_pool, len)?;
            io::Result::Ok(rt.submit(op))
        })?
        .await;
        unsafe { res.take_buffer() }
    }
}

impl AsyncRead for ChildStderr {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Read::new(fd, buffer);
        let res = compio_runtime::submit(op).await.into_inner();
        unsafe { res.map_advanced() }
    }
}

impl AsyncReadManaged for ChildStderr {
    type Buffer = BufferRef;

    async fn read_managed(&mut self, len: usize) -> io::Result<Option<Self::Buffer>> {
        let fd = self.to_shared_fd();
        let res = Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = ReadManaged::new(fd, &buffer_pool, len)?;
            io::Result::Ok(rt.submit(op))
        })?
        .await;
        unsafe { res.take_buffer() }
    }
}

impl AsyncWrite for ChildStdin {
    async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = Write::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner()
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}
