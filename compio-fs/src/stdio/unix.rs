use std::{io, mem::ManuallyDrop};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::{FromRawFd, RawFd};
use compio_io::{AsyncRead, AsyncWrite};
use compio_runtime::TryAsRawFd;

use crate::pipe::{Receiver, Sender};

/// A handle to the standard input stream of a process.
///
/// See [`stdin`].
pub struct Stdin(ManuallyDrop<Receiver>);

impl Stdin {
    pub(crate) fn new() -> Self {
        // SAFETY: we don't drop it
        Self(ManuallyDrop::new(unsafe {
            Receiver::from_raw_fd(libc::STDIN_FILENO)
        }))
    }
}

impl AsyncRead for Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.0.read_vectored(buf).await
    }
}

impl TryAsRawFd for Stdin {
    fn try_as_raw_fd(&self) -> io::Result<RawFd> {
        self.0.try_as_raw_fd()
    }

    unsafe fn as_raw_fd_unchecked(&self) -> RawFd {
        self.0.as_raw_fd_unchecked()
    }
}

/// A handle to the standard output stream of a process.
///
/// See [`stdout`].
pub struct Stdout(ManuallyDrop<Sender>);

impl Stdout {
    pub(crate) fn new() -> Self {
        // SAFETY: we don't drop it
        Self(ManuallyDrop::new(unsafe {
            Sender::from_raw_fd(libc::STDOUT_FILENO)
        }))
    }
}

impl AsyncWrite for Stdout {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

impl TryAsRawFd for Stdout {
    fn try_as_raw_fd(&self) -> io::Result<RawFd> {
        self.0.try_as_raw_fd()
    }

    unsafe fn as_raw_fd_unchecked(&self) -> RawFd {
        self.0.as_raw_fd_unchecked()
    }
}

/// A handle to the standard output stream of a process.
///
/// See [`stderr`].
pub struct Stderr(ManuallyDrop<Sender>);

impl Stderr {
    pub(crate) fn new() -> Self {
        // SAFETY: we don't drop it
        Self(ManuallyDrop::new(unsafe {
            Sender::from_raw_fd(libc::STDERR_FILENO)
        }))
    }
}

impl AsyncWrite for Stderr {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write_vectored(buf).await
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

impl TryAsRawFd for Stderr {
    fn try_as_raw_fd(&self) -> io::Result<RawFd> {
        self.0.try_as_raw_fd()
    }

    unsafe fn as_raw_fd_unchecked(&self) -> RawFd {
        self.0.as_raw_fd_unchecked()
    }
}
