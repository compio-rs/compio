use std::{io, panic::resume_unwind, process};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    op::{BufResultExt, Recv, RecvBufferPool, Send},
    AsRawFd, RawFd, SharedFd, TakeBuffer, ToSharedFd,
};
use compio_io::{AsyncRead, AsyncReadBufferPool, AsyncWrite};
use compio_runtime::buffer_pool::{BorrowedBuffer, BufferPool};

use crate::{ChildStderr, ChildStdin, ChildStdout};

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    compio_runtime::spawn_blocking(move || child.wait())
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

impl AsRawFd for ChildStdout {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl ToSharedFd<process::ChildStdout> for ChildStdout {
    fn to_shared_fd(&self) -> SharedFd<process::ChildStdout> {
        self.0.to_shared_fd()
    }
}

impl AsyncRead for ChildStdout {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }
}

impl AsyncReadBufferPool for ChildStdout {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_buffer_pool<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        let fd = self.to_shared_fd();
        let op = RecvBufferPool::new(buffer_pool.as_driver_buffer_pool(), fd, len as _)?;
        let (BufResult(res, op), flags) = compio_runtime::submit_with_flags(op).await;

        op.take_buffer(buffer_pool.as_driver_buffer_pool(), res, flags)
    }
}

impl AsRawFd for ChildStderr {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl ToSharedFd<process::ChildStderr> for ChildStderr {
    fn to_shared_fd(&self) -> SharedFd<process::ChildStderr> {
        self.0.to_shared_fd()
    }
}

impl AsyncRead for ChildStderr {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }
}

impl AsyncReadBufferPool for ChildStderr {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_buffer_pool<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        let fd = self.to_shared_fd();
        let op = RecvBufferPool::new(buffer_pool.as_driver_buffer_pool(), fd, len as _)?;
        let (BufResult(res, op), flags) = compio_runtime::submit_with_flags(op).await;

        op.take_buffer(buffer_pool.as_driver_buffer_pool(), res, flags)
    }
}

impl AsRawFd for ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl ToSharedFd<process::ChildStdin> for ChildStdin {
    fn to_shared_fd(&self) -> SharedFd<process::ChildStdin> {
        self.0.to_shared_fd()
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
