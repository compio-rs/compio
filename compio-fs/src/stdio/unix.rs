use std::mem::ManuallyDrop;

use crate::pipe::{Receiver, Sender};
use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_driver::AsRawFd;
use compio_io::{AsyncRead, AsyncWrite};
use compio_runtime::FromRawFd;

/// A handle to the standard input stream of a process.
///
/// See [`stdin`].
pub struct Stdin(ManuallyDrop<Receiver>);

impl Stdin {
    pub(crate) fn new() -> Self {
        Self(ManuallyDrop::new(unsafe {
            Receiver::from_raw_fd(std::io::stdin().as_raw_fd())
        }))
    }
}

impl AsyncRead for Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.read(buf).await
    }
}

/// A handle to the standard output stream of a process.
///
/// See [`stdout`].
pub struct Stdout(ManuallyDrop<Sender>);

impl Stdout {
    pub(crate) fn new() -> Self {
        Self(ManuallyDrop::new(unsafe {
            Sender::from_raw_fd(std::io::stdout().as_raw_fd())
        }))
    }
}

impl AsyncWrite for Stdout {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        self.0.shutdown().await
    }
}

/// A handle to the standard output stream of a process.
///
/// See [`stderr`].
pub struct Stderr(ManuallyDrop<Sender>);

impl Stderr {
    pub(crate) fn new() -> Self {
        Self(ManuallyDrop::new(unsafe {
            Sender::from_raw_fd(std::io::stderr().as_raw_fd())
        }))
    }
}

impl AsyncWrite for Stderr {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        self.0.shutdown().await
    }
}
