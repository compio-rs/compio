use std::{io, panic::resume_unwind, process};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    AsRawFd, RawFd, SharedFd, ToSharedFd,
    op::{BufResultExt, Recv, Send},
};
use compio_io::{AsyncRead, AsyncWrite};

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
