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
    OpCode, RawFd,
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

/// Constructs a new handle to the standard input of the current process.
///
/// The handle implements the [`AsyncRead`] trait, but beware that concurrent
/// reads of [`Stdin`] must be executed with care.
///
/// This handle is best used for non-interactive uses, such as when a file
/// is piped into the application. For technical reasons, if `stdin` is a
/// console handle, the read method is implemented by using an ordinary blocking
/// read on a separate thread, and it is impossible to cancel that read. This
/// can make shutdown of the runtime hang until the user presses enter.
pub fn stdin() -> Stdin {
    let stdin = std::io::stdin();
    let isatty = *STDIN_ISATTY.get_or_init(|| {
        stdin.is_terminal() || Runtime::current().attach(stdin.as_raw_handle()).is_err()
    });
    Stdin {
        fd: stdin.as_raw_handle() as _,
        isatty,
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

static STDOUT_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard output stream of a process.
///
/// See [`stdout`].
pub struct Stdout {
    fd: RawFd,
    isatty: bool,
}

/// Constructs a new handle to the standard output of the current process.
///
/// Concurrent writes to stdout must be executed with care: Only individual
/// writes to this [`AsyncWrite`] are guaranteed to be intact. In particular
/// you should be aware that writes using [`write_all`] are not guaranteed
/// to occur as a single write, so multiple threads writing data with
/// [`write_all`] may result in interleaved output.
///
/// [`write_all`]: compio_io::AsyncWriteExt::write_all
pub fn stdout() -> Stdout {
    let stdout = std::io::stdout();
    let isatty = *STDOUT_ISATTY.get_or_init(|| {
        stdout.is_terminal() || Runtime::current().attach(stdout.as_raw_handle()).is_err()
    });
    Stdout {
        fd: stdout.as_raw_handle() as _,
        isatty,
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

static STDERR_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard output stream of a process.
///
/// See [`stderr`].
pub struct Stderr {
    fd: RawFd,
    isatty: bool,
}

/// Constructs a new handle to the standard output of the current process.
///
/// Concurrent writes to stderr must be executed with care: Only individual
/// writes to this [`AsyncWrite`] are guaranteed to be intact. In particular
/// you should be aware that writes using [`write_all`] are not guaranteed
/// to occur as a single write, so multiple threads writing data with
/// [`write_all`] may result in interleaved output.
///
/// [`write_all`]: compio_io::AsyncWriteExt::write_all
pub fn stderr() -> Stderr {
    let stderr = std::io::stderr();
    let isatty = *STDERR_ISATTY.get_or_init(|| {
        stderr.is_terminal() || Runtime::current().attach(stderr.as_raw_handle()).is_err()
    });
    Stderr {
        fd: stderr.as_raw_handle() as _,
        isatty,
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
