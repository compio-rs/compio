use std::{
    io::{self, IsTerminal, Read, Write},
    os::windows::io::AsRawHandle,
    pin::Pin,
    sync::OnceLock,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    op::{BufResultExt, Recv, Send},
    AsRawFd, OpCode, RawFd,
};
use compio_io::{AsyncRead, AsyncWrite};
use compio_runtime::Runtime;
use windows_sys::Win32::System::IO::OVERLAPPED;

struct StdRead<R: Read, B: IoBufMut> {
    reader: R,
    buffer: B,
}

impl<R: Read, B: IoBufMut> StdRead<R, B> {
    pub fn new(reader: R, buffer: B) -> Self {
        Self { reader, buffer }
    }
}

impl<R: Read, B: IoBufMut> OpCode for StdRead<R, B> {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let slice = this.buffer.as_mut_slice();
        #[cfg(feature = "read_buf")]
        {
            use std::io::BorrowedBuf;

            let mut buf = BorrowedBuf::from(slice);
            let mut cursor = buf.unfilled();
            this.reader.read_buf(cursor.reborrow())?;
            Poll::Ready(Ok(cursor.written()))
        }
        #[cfg(not(feature = "read_buf"))]
        {
            use std::mem::MaybeUninit;

            slice.fill(MaybeUninit::new(0));
            this.reader
                .read(std::slice::from_raw_parts_mut(
                    this.buffer.as_buf_mut_ptr(),
                    this.buffer.buf_capacity(),
                ))
                .into()
        }
    }
}

impl<R: Read, B: IoBufMut> IntoInner for StdRead<R, B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

struct StdWrite<W: Write, B: IoBuf> {
    writer: W,
    buffer: B,
}

impl<W: Write, B: IoBuf> StdWrite<W, B> {
    pub fn new(writer: W, buffer: B) -> Self {
        Self { writer, buffer }
    }
}

impl<W: Write, B: IoBuf> OpCode for StdWrite<W, B> {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(
        self: Pin<&mut Self>,
        _optr: *mut OVERLAPPED,
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let slice = this.buffer.as_slice();
        this.writer.write(slice).into()
    }
}

impl<W: Write, B: IoBuf> IntoInner for StdWrite<W, B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

static STDIN_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard input stream of a process.
///
/// See [`stdin`].
pub struct Stdin {
    fd: RawFd,
    isatty: bool,
}

impl Stdin {
    pub(crate) fn new() -> Self {
        let stdin = std::io::stdin();
        let isatty = *STDIN_ISATTY.get_or_init(|| {
            stdin.is_terminal() || Runtime::current().attach(stdin.as_raw_handle()).is_err()
        });
        Self {
            fd: stdin.as_raw_handle() as _,
            isatty,
        }
    }
}

impl AsyncRead for Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        let runtime = Runtime::current();
        if self.isatty {
            let op = StdRead::new(std::io::stdin(), buf);
            runtime.submit(op).await.into_inner()
        } else {
            let op = Recv::new(self.fd, buf);
            runtime.submit(op).await.into_inner()
        }
        .map_advanced()
    }
}

impl AsRawFd for Stdin {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

static STDOUT_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard output stream of a process.
///
/// See [`stdout`].
pub struct Stdout {
    fd: RawFd,
    isatty: bool,
}

impl Stdout {
    pub(crate) fn new() -> Self {
        let stdout = std::io::stdout();
        let isatty = *STDOUT_ISATTY.get_or_init(|| {
            stdout.is_terminal() || Runtime::current().attach(stdout.as_raw_handle()).is_err()
        });
        Self {
            fd: stdout.as_raw_handle() as _,
            isatty,
        }
    }
}

impl AsyncWrite for Stdout {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let runtime = Runtime::current();
        if self.isatty {
            let op = StdWrite::new(std::io::stdout(), buf);
            runtime.submit(op).await.into_inner()
        } else {
            let op = Send::new(self.fd, buf);
            runtime.submit(op).await.into_inner()
        }
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        self.flush().await
    }
}

impl AsRawFd for Stdout {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

static STDERR_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard output stream of a process.
///
/// See [`stderr`].
pub struct Stderr {
    fd: RawFd,
    isatty: bool,
}

impl Stderr {
    pub(crate) fn new() -> Self {
        let stderr = std::io::stderr();
        let isatty = *STDERR_ISATTY.get_or_init(|| {
            stderr.is_terminal() || Runtime::current().attach(stderr.as_raw_handle()).is_err()
        });
        Self {
            fd: stderr.as_raw_handle() as _,
            isatty,
        }
    }
}

impl AsyncWrite for Stderr {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let runtime = Runtime::current();
        if self.isatty {
            let op = StdWrite::new(std::io::stderr(), buf);
            runtime.submit(op).await.into_inner()
        } else {
            let op = Send::new(self.fd, buf);
            runtime.submit(op).await.into_inner()
        }
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        self.flush().await
    }
}

impl AsRawFd for Stderr {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}
