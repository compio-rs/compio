use std::{io, process};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    BufferRef, ResultTakeBuffer, ToSharedFd,
    op::{BufResultExt, Read, ReadManaged, Write},
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};
use compio_runtime::{ResumeUnwind, Runtime, SpawnMeta};

use crate::{ChildStderr, ChildStdin, ChildStdout};

pub async fn child_wait(mut child: process::Child) -> io::Result<process::ExitStatus> {
    // Name the task: its location points here rather than into the code that
    // waited for the child, since this is an `async fn`.
    let meta = SpawnMeta::capture().named("process::wait");
    compio_runtime::spawn_blocking_at(move || child.wait(), meta)
        .await
        .resume_unwind()
        .expect("shouldn't be cancelled")
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
