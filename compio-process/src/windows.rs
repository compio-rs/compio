use std::{
    io,
    os::windows::{io::AsRawHandle, process::ExitStatusExt},
    pin::Pin,
    process,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    OpCode, OpType, ToSharedFd,
    op::{BufResultExt, Recv, RecvManaged, ResultTakeBuffer, Send},
    syscall,
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};
use compio_runtime::{BorrowedBuffer, BufferPool};
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

impl OpCode for WaitProcess {
    fn op_type(&self) -> OpType {
        OpType::Event(self.child.as_raw_handle() as _)
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
    let code = compio_runtime::submit(op).await.0?;
    Ok(process::ExitStatus::from_raw(code as _))
}

impl AsyncRead for ChildStdout {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }
}

impl AsyncReadManaged for ChildStdout {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        let fd = self.to_shared_fd();
        let buffer_pool = buffer_pool.try_inner()?;
        let op = RecvManaged::new(fd, buffer_pool, len)?;
        compio_runtime::submit_with_flags(op)
            .await
            .take_buffer(buffer_pool)
    }
}

impl AsyncRead for ChildStderr {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }
}

impl AsyncReadManaged for ChildStderr {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        let fd = self.to_shared_fd();
        let buffer_pool = buffer_pool.try_inner()?;
        let op = RecvManaged::new(fd, buffer_pool, len)?;
        compio_runtime::submit_with_flags(op)
            .await
            .take_buffer(buffer_pool)
    }
}

impl AsyncWrite for ChildStdin {
    async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = Send::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner()
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}
