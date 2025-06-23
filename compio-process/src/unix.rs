use std::{io, panic::resume_unwind, process};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    ToSharedFd,
    op::{BufResultExt, Recv, RecvManaged, ResultTakeBuffer, Send},
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};
use compio_runtime::{BorrowedBuffer, BufferPool};

use crate::{ChildStderr, ChildStdin, ChildStdout};

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    compio_runtime::spawn_blocking(move || child.wait())
        .await
        .unwrap_or_else(|e| resume_unwind(e))
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
