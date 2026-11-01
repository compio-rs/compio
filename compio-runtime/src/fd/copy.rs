use std::{
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    SharedFd,
    op::{FileStat, Interest, Pipe, PollOnce, Splice, SpliceFlags},
};
use compio_io::{AsyncRead, AsyncWrite, AsyncWriteExt, util::copy_buffered};

use crate::{Runtime, fd::AsyncFd};

// Bound each splice and amortize worker dispatch over eight fill/drain pairs.
const SPLICE_PAIRS: usize = 8;
const BUFFER_SIZE: usize = 8192;

// Every in-flight operation retains both pipe ends and both external handles.
// Closing the opposite pipe end during cancellation can block on a pipe mutex
// held by a pending splice, preventing the executor from servicing
// cancellation.
struct CopyFds<I, O> {
    input: I,
    output: O,
    rx: OwnedFd,
    tx: OwnedFd,
}

impl<I, O> AsFd for CopyFds<I, O> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.rx.as_fd()
    }
}

#[derive(Clone, Copy)]
enum CopyEnd {
    Input,
    Output,
    PipeReader,
    PipeWriter,
}

struct CopyFd<I, O> {
    fds: SharedFd<CopyFds<I, O>>,
    end: CopyEnd,
}

impl<I: AsFd, O: AsFd> AsFd for CopyFd<I, O> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self.end {
            CopyEnd::Input => self.fds.input.as_fd(),
            CopyEnd::Output => self.fds.output.as_fd(),
            CopyEnd::PipeReader => self.fds.rx.as_fd(),
            CopyEnd::PipeWriter => self.fds.tx.as_fd(),
        }
    }
}

/// Internal specialization for unbuffered, current-position byte streams.
///
/// `fd` must own the reader's descriptor and have the same read semantics as
/// `reader`. Operations retain ownership even when this future is cancelled.
#[doc(hidden)]
pub async fn copy_splice<R, W, S>(
    reader: &mut R,
    writer: &mut W,
    input: S,
    buf_size: Option<usize>,
) -> io::Result<u64>
where
    R: AsyncRead + ?Sized,
    W: AsyncWrite + ?Sized,
    S: AsFd + Clone + 'static,
{
    // Keep zero-sized copies on the buffered path. F_SETPIPE_SZ takes a
    // positive signed int; do not truncate larger requested sizes.
    if buf_size.is_some_and(|size| size == 0 || i32::try_from(size).is_err())
        || !Runtime::with_current(|rt| rt.driver_type().is_iouring())
    {
        return copy_buffered(reader, writer, buf_size.unwrap_or(BUFFER_SIZE)).await;
    }
    let Some(output) = writer.copy_fd() else {
        return copy_buffered(reader, writer, buf_size.unwrap_or(BUFFER_SIZE)).await;
    };
    let buffer_size = buf_size.unwrap_or(BUFFER_SIZE);

    // Each pair moves bytes from input -> pipe, then pipe -> output.
    // If the output accepts only some bytes, the pipe still holds the rest.
    // Reading more input now could deadlock: the producer may be waiting for
    // those remaining bytes to reach the output before it sends anything else.
    //
    // For blocking inputs, stop the batch after an incomplete output splice
    // (a soft link), so we can drain the pipe before reading more input.
    // Nonblocking inputs return WouldBlock instead of waiting, so the batch
    // can continue after that splice (a hard link).
    // Check the existing flags; do not change flags shared with other fd users.
    let nonblocking = rustix::fs::fcntl_getfl(&input)?.contains(rustix::fs::OFlags::NONBLOCK);
    // FIFO EOF is not permanent: a new writer can connect after a zero read.
    // Stop that chain at its first short fill rather than reading past EOF.
    let BufResult(result, stat) = crate::submit(FileStat::new(input.clone())).await;
    if result.is_err() {
        return copy_buffered(reader, writer, buffer_size).await;
    }
    let pipe_input = match rustix::fs::FileType::from_raw_mode(stat.into_inner().stat.st_mode) {
        rustix::fs::FileType::Fifo => true,
        rustix::fs::FileType::Socket
            if rustix::net::sockopt::socket_type(&input) == Ok(rustix::net::SocketType::STREAM) =>
        {
            false
        }
        // Seekable AsyncFd offsets differ between backends; preserve its
        // ordinary read semantics rather than splice's current-position IO.
        _ => return copy_buffered(reader, writer, buffer_size).await,
    };
    let hardlinks = std::array::from_fn(|i| {
        let is_pipe_fill = i % 2 == 0;
        if is_pipe_fill {
            !pipe_input
        } else {
            nonblocking
        }
    });
    let BufResult(result, pipe) = crate::submit(Pipe::new()).await;
    if result.is_err() {
        return copy_buffered(reader, writer, buffer_size).await;
    }
    let (rx, tx) = pipe.into_inner();
    // The kernel enforces pipe-max-size, capabilities, and per-user quotas.
    // With no explicit size, preserve the capacity chosen at pipe creation.
    // Requests below the kernel default are slower than an equally sized
    // userspace buffer (measured ~1.7x in bulk), so keep those buffered.
    let Ok(default_capacity) = rustix::pipe::fcntl_getpipe_size(&rx) else {
        return copy_buffered(reader, writer, buffer_size).await;
    };
    let splice_size = match buf_size {
        Some(size) if size >= default_capacity => {
            let Ok(capacity) = rustix::pipe::fcntl_setpipe_size(&rx, size) else {
                return copy_buffered(reader, writer, buffer_size).await;
            };
            size.min(capacity)
        }
        Some(_) => return copy_buffered(reader, writer, buffer_size).await,
        None => default_capacity,
    };
    // Use 4 KiB of empirical accounting headroom for small send buffers.
    // One large TCP segment can otherwise exhaust that buffer and make each
    // writable wakeup depend on a delayed ACK. Keep the pipe capacity
    // unchanged.
    let splice_size = if let Ok(sndbuf) = rustix::net::sockopt::socket_send_buffer_size(&output) {
        splice_size.min(sndbuf.saturating_sub(4 * 1024).max(1))
    } else {
        splice_size
    };
    let fds = SharedFd::new(CopyFds {
        input,
        output,
        rx,
        tx,
    });
    let fd = |end| CopyFd {
        fds: fds.clone(),
        end,
    };
    let mut rx = fd(CopyEnd::PipeReader);
    let mut output = fd(CopyEnd::Output);
    let mut total = 0;
    let mut ops: [_; SPLICE_PAIRS * 2] = std::array::from_fn(|i| {
        let is_pipe_fill = i % 2 == 0;
        let (input, output) = if is_pipe_fill {
            (CopyEnd::Input, CopyEnd::PipeWriter)
        } else {
            (CopyEnd::PipeReader, CopyEnd::Output)
        };
        Splice::new(
            fd(input),
            -1,
            fd(output),
            -1,
            splice_size,
            SpliceFlags::NONBLOCK,
        )
    });
    loop {
        // The driver publishes the entire chain together. Await every member,
        // even after errors, before accounting for bytes or draining the pipe.
        let mut results = crate::submit_linked(ops, hardlinks).await;

        let mut bytes_read_this_batch = 0;
        let mut pending = 0usize;
        let mut eof = false;
        let mut input_would_block = false;
        let mut fallback = false;
        let mut failure = None;
        let mut cancelled_tail = false;
        for (i, BufResult(result, _)) in results.iter_mut().enumerate() {
            let is_pipe_fill = i % 2 == 0;
            let short =
                !matches!(&result, Ok(bytes_transferred) if *bytes_transferred == splice_size);
            let result = std::mem::replace(result, Ok(0));
            match result {
                Ok(bytes_read) if is_pipe_fill => {
                    eof |= bytes_read == 0;
                    bytes_read_this_batch += bytes_read;
                    pending += bytes_read;
                }
                Ok(bytes_written) => {
                    if bytes_written == 0 && pending != 0 {
                        failure.get_or_insert_with(|| io::ErrorKind::WriteZero.into());
                    }
                    pending = pending.checked_sub(bytes_written).ok_or_else(|| {
                        io::Error::other("splice drained more bytes than were read")
                    })?;
                    total += bytes_written as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    input_would_block |= is_pipe_fill;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if unsupported(&e) => fallback = true,
                Err(e)
                    if e.raw_os_error() == Some(libc::ECANCELED)
                        && (cancelled_tail || fallback || failure.is_some()) => {}
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
            cancelled_tail |= short && !hardlinks[i] && i + 1 < SPLICE_PAIRS * 2;
        }
        // Submission returns ownership of every completed operation.
        ops = results.map(|BufResult(_, op)| op);
        if let Some(error) = failure {
            return Err(error);
        }

        // Every batch starts with an empty pipe. Drain any residual bytes
        // before consuming more input, and preserve them if splice must
        // fall back.
        while pending != 0 && !fallback {
            let result;
            BufResult(result, (rx, output)) =
                splice(rx, output, pending.min(splice_size), Interest::Writable).await;
            match result {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(bytes_written) => {
                    pending -= bytes_written;
                    total += bytes_written as u64;
                }
                Err(e) if unsupported(&e) => fallback = true,
                Err(e) => return Err(e),
            }
        }
        if fallback {
            // Splice is unsupported for these endpoints. Bytes already read
            // from the input may still be in our private pipe; switching
            // directly to copy_buffered(reader, ...) would lose those bytes.
            // Forward them through a userspace buffer first, then copy the
            // rest from the original reader, preserving byte order and count.
            if pending != 0 {
                let mut pipe = AsyncFd::new(rx)?;
                let mut buf = Vec::with_capacity(pending.min(buffer_size));
                while pending != 0 {
                    let result;
                    BufResult(result, buf) = pipe.read(buf).await;
                    let bytes_read = match result {
                        Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                        Ok(bytes_read) => bytes_read,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    };
                    let result;
                    BufResult(result, buf) = writer.write_all(buf).await;
                    result?;
                    pending -= bytes_read;
                    total += bytes_read as u64;
                    buf.clear();
                }
            }
            return Ok(total + copy_buffered(reader, writer, buffer_size).await?);
        }
        if eof {
            writer.flush().await?;
            writer.shutdown().await?;
            return Ok(total);
        }
        // No input progress: wait for readability instead of repeatedly
        // retrying.
        if bytes_read_this_batch == 0 && input_would_block {
            let BufResult(result, _) =
                crate::submit(PollOnce::new(fd(CopyEnd::Input), Interest::Readable)).await;
            match result {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

async fn splice<I: AsFd + 'static, O: AsFd + 'static>(
    mut input: I,
    mut output: O,
    len: usize,
    interest: Interest,
) -> BufResult<usize, (I, O)> {
    loop {
        let result;
        BufResult(result, (input, output)) = crate::submit(Splice::new(
            input,
            -1,
            output,
            -1,
            len,
            SpliceFlags::NONBLOCK,
        ))
        .await
        .into_inner();
        if matches!(&result, Err(e) if e.kind() == io::ErrorKind::Interrupted) {
            continue;
        }
        if matches!(&result, Err(e) if e.kind() == io::ErrorKind::WouldBlock) {
            // The private pipe is empty on input and nonempty on output, so
            // only the external endpoint can need readiness. Imported sockets
            // may retain O_NONBLOCK even under the io_uring backend.
            let ready;
            match interest {
                Interest::Readable => {
                    BufResult(ready, input) = crate::submit(PollOnce::new(input, interest))
                        .await
                        .into_inner();
                }
                Interest::Writable => {
                    BufResult(ready, output) = crate::submit(PollOnce::new(output, interest))
                        .await
                        .into_inner();
                }
            }
            match ready {
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return BufResult(Err(e), (input, output)),
            }
        }
        return BufResult(result, (input, output));
    }
}

fn unsupported(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported
        || matches!(
            error.raw_os_error(),
            Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP | libc::EXDEV)
        )
}
