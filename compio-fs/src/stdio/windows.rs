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
    AsRawFd, OpCode, OpType, RawFd, SharedFd,
};
use compio_io::{AsyncRead, AsyncWrite};
use compio_runtime::Runtime;
use windows_sys::Win32::System::IO::OVERLAPPED;

#[cfg(doc)]
use super::{stderr, stdin, stdout};

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
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let slice = this.buffer.as_mut_slice();
        #[cfg(feature = "read_buf")]
        {
            let mut buf = io::BorrowedBuf::from(slice);
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
    fn op_type(&self) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
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
#[derive(Debug, Clone)]
pub struct Stdin {
    fd: SharedFd<RawFd>,
    isatty: bool,
}

impl Stdin {
    pub(crate) fn new() -> Self {
        let stdin = io::stdin();
        let isatty = *STDIN_ISATTY.get_or_init(|| {
            stdin.is_terminal()
                || Runtime::current()
                    .attach(stdin.as_raw_handle() as _)
                    .is_err()
        });
        Self {
            fd: SharedFd::new(stdin.as_raw_handle() as _),
            isatty,
        }
    }
}

impl AsyncRead for Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        let runtime = Runtime::current();
        if self.isatty {
            let op = StdRead::new(io::stdin(), buf);
            runtime.submit(op).await.into_inner()
        } else {
            let op = Recv::new(self.fd.clone(), buf);
            runtime.submit(op).await.into_inner()
        }
        .map_advanced()
    }
}

impl AsRawFd for Stdin {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

static STDOUT_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard output stream of a process.
///
/// See [`stdout`].
#[derive(Debug, Clone)]
pub struct Stdout {
    fd: SharedFd<RawFd>,
    isatty: bool,
}

impl Stdout {
    pub(crate) fn new() -> Self {
        let stdout = io::stdout();
        let isatty = *STDOUT_ISATTY.get_or_init(|| {
            stdout.is_terminal()
                || Runtime::current()
                    .attach(stdout.as_raw_handle() as _)
                    .is_err()
        });
        Self {
            fd: SharedFd::new(stdout.as_raw_handle() as _),
            isatty,
        }
    }
}

impl AsyncWrite for Stdout {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let runtime = Runtime::current();
        if self.isatty {
            let op = StdWrite::new(io::stdout(), buf);
            runtime.submit(op).await.into_inner()
        } else {
            let op = Send::new(self.fd.clone(), buf);
            runtime.submit(op).await.into_inner()
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.flush().await
    }
}

impl AsRawFd for Stdout {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

static STDERR_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard output stream of a process.
///
/// See [`stderr`].
#[derive(Debug, Clone)]
pub struct Stderr {
    fd: SharedFd<RawFd>,
    isatty: bool,
}

impl Stderr {
    pub(crate) fn new() -> Self {
        let stderr = io::stderr();
        let isatty = *STDERR_ISATTY.get_or_init(|| {
            stderr.is_terminal()
                || Runtime::current()
                    .attach(stderr.as_raw_handle() as _)
                    .is_err()
        });
        Self {
            fd: SharedFd::new(stderr.as_raw_handle() as _),
            isatty,
        }
    }
}

impl AsyncWrite for Stderr {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let runtime = Runtime::current();
        if self.isatty {
            let op = StdWrite::new(io::stderr(), buf);
            runtime.submit(op).await.into_inner()
        } else {
            let op = Send::new(self.fd.clone(), buf);
            runtime.submit(op).await.into_inner()
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.flush().await
    }
}

impl AsRawFd for Stderr {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}
