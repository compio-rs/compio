use std::{
    io::{self, IsTerminal, Read, Write},
    os::windows::io::{AsRawHandle, BorrowedHandle, RawHandle},
    pin::Pin,
    sync::OnceLock,
    task::Poll,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, OpCode, OpType, RawFd, SharedFd,
    op::{BufResultExt, Recv, RecvManaged, ResultTakeBuffer, Send},
};
use compio_io::{AsyncRead, AsyncReadManaged, AsyncWrite};
use compio_runtime::{BorrowedBuffer, BufferPool, Runtime};
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
        let this = unsafe { self.get_unchecked_mut() };
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
        let this = unsafe { self.get_unchecked_mut() };
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

#[derive(Debug)]
struct StaticFd(RawHandle);

impl AsFd for StaticFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: we only use it for console handles.
        BorrowedFd::File(unsafe { BorrowedHandle::borrow_raw(self.0) })
    }
}

impl AsRawFd for StaticFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0 as _
    }
}

static STDIN_ISATTY: OnceLock<bool> = OnceLock::new();

/// A handle to the standard input stream of a process.
///
/// See [`stdin`].
#[derive(Debug, Clone)]
pub struct Stdin {
    fd: SharedFd<StaticFd>,
    isatty: bool,
}

impl Stdin {
    pub(crate) fn new() -> Self {
        let stdin = io::stdin();
        let isatty = *STDIN_ISATTY.get_or_init(|| {
            stdin.is_terminal()
                || Runtime::with_current(|r| r.attach(stdin.as_raw_handle() as _)).is_err()
        });
        Self {
            fd: SharedFd::new(StaticFd(stdin.as_raw_handle())),
            isatty,
        }
    }
}

impl AsyncRead for Stdin {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        if self.isatty {
            let op = StdRead::new(io::stdin(), buf);
            compio_runtime::submit(op).await.into_inner()
        } else {
            let op = Recv::new(self.fd.clone(), buf);
            compio_runtime::submit(op).await.into_inner()
        }
        .map_advanced()
    }
}

impl AsyncReadManaged for Stdin {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        (&*self).read_managed(buffer_pool, len).await
    }
}

impl AsyncReadManaged for &Stdin {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed<'a>(
        &mut self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
    ) -> io::Result<Self::Buffer<'a>> {
        let buffer_pool = buffer_pool.try_inner()?;
        if self.isatty {
            let buf = buffer_pool.get_buffer(len)?;
            let op = StdRead::new(io::stdin(), buf);
            let BufResult(res, buf) = compio_runtime::submit(op).await.into_inner();
            let res = unsafe { buffer_pool.create_proxy(buf, res?) };
            Ok(res)
        } else {
            let op = RecvManaged::new(self.fd.clone(), buffer_pool, len)?;
            compio_runtime::submit_with_flags(op)
                .await
                .take_buffer(buffer_pool)
        }
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
    fd: SharedFd<StaticFd>,
    isatty: bool,
}

impl Stdout {
    pub(crate) fn new() -> Self {
        let stdout = io::stdout();
        let isatty = *STDOUT_ISATTY.get_or_init(|| {
            stdout.is_terminal()
                || Runtime::with_current(|r| r.attach(stdout.as_raw_handle() as _)).is_err()
        });
        Self {
            fd: SharedFd::new(StaticFd(stdout.as_raw_handle())),
            isatty,
        }
    }
}

impl AsyncWrite for Stdout {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        if self.isatty {
            let op = StdWrite::new(io::stdout(), buf);
            compio_runtime::submit(op).await.into_inner()
        } else {
            let op = Send::new(self.fd.clone(), buf);
            compio_runtime::submit(op).await.into_inner()
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
    fd: SharedFd<StaticFd>,
    isatty: bool,
}

impl Stderr {
    pub(crate) fn new() -> Self {
        let stderr = io::stderr();
        let isatty = *STDERR_ISATTY.get_or_init(|| {
            stderr.is_terminal()
                || Runtime::with_current(|r| r.attach(stderr.as_raw_handle() as _)).is_err()
        });
        Self {
            fd: SharedFd::new(StaticFd(stderr.as_raw_handle())),
            isatty,
        }
    }
}

impl AsyncWrite for Stderr {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        if self.isatty {
            let op = StdWrite::new(io::stderr(), buf);
            compio_runtime::submit(op).await.into_inner()
        } else {
            let op = Send::new(self.fd.clone(), buf);
            compio_runtime::submit(op).await.into_inner()
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
