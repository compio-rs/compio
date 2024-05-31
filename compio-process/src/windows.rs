use std::{
    io,
    os::windows::{io::AsRawHandle, process::ExitStatusExt},
    pin::Pin,
    process,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    op::{BufResultExt, Recv, Send},
    syscall, AsRawFd, OpCode, OpType, RawFd, SharedFd, ToSharedFd,
};
use compio_io::{AsyncRead, AsyncWrite};
use windows_sys::Win32::System::{Threading::GetExitCodeProcess, IO::OVERLAPPED};

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
