use std::{
    io::{self, BufRead, Read, Write},
    mem::MaybeUninit,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};

use crate::{
    buffer::Buffer,
    util::{DEFAULT_BUF_SIZE, Splittable},
};

// 64MiB max
pub(crate) const DEFAULT_MAX_BUFFER: usize = 64 * 1024 * 1024;

#[derive(Debug)]
struct SyncReadBuf {
    buf: Buffer,
    eof: bool,
    base_capacity: usize,
    max_buffer_size: usize,
}

impl SyncReadBuf {
    pub fn new(start_capacity: usize, base_capacity: usize, max_buffer_size: usize) -> Self {
        Self {
            buf: Buffer::with_capacity(start_capacity),
            eof: false,
            base_capacity,
            max_buffer_size,
        }
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    pub fn into_inner(mut self) -> Vec<u8> {
        if self.buf.has_inner() {
            let slice = self.buf.take_inner();
            let begin = slice.begin();
            let mut vec = slice.into_inner();
            if begin > 0 {
                vec.drain(..begin);
            }
            vec
        } else {
            Vec::new()
        }
    }

    /// Returns the available bytes in the read buffer.
    fn available_read(&self) -> io::Result<&[u8]> {
        if self.buf.has_inner() {
            Ok(self.buf.buffer())
        } else {
            Err(would_block("the read buffer is in use"))
        }
    }

    /// Marks `amt` bytes as consumed from the read buffer.
    ///
    /// Resets the buffer when all data is consumed and shrinks capacity
    /// if it has grown significantly beyond the base capacity.
    pub fn consume(&mut self, amt: usize) {
        let all_done = self.buf.advance(amt);

        // Shrink oversized buffers back to base capacity
        if all_done {
            self.buf
                .compact_to(self.base_capacity, self.max_buffer_size);
        }
    }

    pub fn read_buf_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        let available = self.fill_buf()?;

        let to_read = available.len().min(buf.len());
        buf[..to_read].copy_from_slice(unsafe {
            std::slice::from_raw_parts(available.as_ptr().cast(), to_read)
        });
        self.consume(to_read);

        Ok(to_read)
    }

    pub fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let available = self.available_read()?;

        if available.is_empty() && !self.eof {
            return Err(would_block("need to fill read buffer"));
        }

        Ok(available)
    }

    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut slice = self.fill_buf()?;
        slice.read(buf).inspect(|res| {
            self.consume(*res);
        })
    }

    #[cfg(feature = "read_buf")]
    pub fn read_buf(&mut self, mut buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        let mut slice = self.fill_buf()?;
        let old_written = buf.written();
        slice.read_buf(buf.reborrow())?;
        let len = buf.written() - old_written;
        self.consume(len);
        Ok(())
    }

    pub async fn fill_read_buf<S: crate::AsyncRead>(
        &mut self,
        stream: &mut S,
    ) -> io::Result<usize> {
        if self.eof {
            return Ok(0);
        }

        // Compact buffer, move unconsumed data to the front
        self.buf
            .compact_to(self.base_capacity, self.max_buffer_size);

        let read = self
            .buf
            .with(|mut inner| async {
                let current_len = inner.buf_len();

                if current_len >= self.max_buffer_size {
                    return BufResult(
                        Err(io::Error::new(
                            io::ErrorKind::OutOfMemory,
                            format!("read buffer size limit ({}) exceeded", self.max_buffer_size),
                        )),
                        inner,
                    );
                }

                let capacity = inner.buf_capacity();
                let available_space = capacity - current_len;

                // If target space is less than base capacity, grow the buffer.
                let target_space = self.base_capacity;
                if available_space < target_space {
                    let new_capacity = current_len + target_space;
                    let _ = inner.reserve_exact(new_capacity - capacity);
                }

                let len = inner.buf_len();
                let read_slice = inner.slice(len..);
                stream.read(read_slice).await.into_inner()
            })
            .await?;
        if read == 0 {
            self.eof = true;
        }
        Ok(read)
    }
}

#[derive(Debug)]
struct SyncWriteBuf {
    buf: Buffer,
    base_capacity: usize,
    max_buffer_size: usize,
}

impl SyncWriteBuf {
    pub fn new(start_capacity: usize, base_capacity: usize, max_buffer_size: usize) -> Self {
        Self {
            buf: Buffer::with_capacity(start_capacity),
            base_capacity,
            max_buffer_size,
        }
    }

    pub fn has_pending_write(&self) -> bool {
        !self.buf.is_empty()
    }

    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.buf.has_inner() {
            return Err(would_block("the write buffer is in use"));
        }
        // Check if we should flush first
        if self.buf.need_flush() && !self.buf.is_empty() {
            return Err(would_block("need to flush write buffer"));
        }

        let written = self.buf.with_sync(|mut inner| {
            let res = (|| {
                if inner.buf_len() + buf.len() > self.max_buffer_size {
                    let space = self.max_buffer_size - inner.buf_len();
                    if space == 0 {
                        Err(would_block("write buffer full, need to flush"))
                    } else {
                        inner.extend_from_slice(&buf[..space])?;
                        Ok(space)
                    }
                } else {
                    inner.extend_from_slice(buf)?;
                    Ok(buf.len())
                }
            })();
            BufResult(res, inner)
        })?;

        Ok(written)
    }

    pub async fn flush_write_buf<S: crate::AsyncWrite>(
        &mut self,
        stream: &mut S,
    ) -> io::Result<usize> {
        let flushed = self.buf.flush_to(stream).await?;
        self.buf
            .compact_to(self.base_capacity, self.max_buffer_size);
        stream.flush().await?;
        Ok(flushed)
    }
}

/// A growable buffered stream adapter that bridges async I/O with sync traits.
///
/// # Buffer Growth Strategy
///
/// - **Read buffer**: Grows as needed to accommodate incoming data, up to
///   `max_buffer_size`
/// - **Write buffer**: Grows as needed for outgoing data, up to
///   `max_buffer_size`
/// - Both buffers shrink back to `base_capacity` when fully consumed and
///   capacity exceeds 4x base
///
/// # Usage Pattern
///
/// The sync `Read` and `Write` implementations will return `WouldBlock` errors
/// when buffers need servicing via the async methods:
///
/// - Call `fill_read_buf()` when `Read::read()` returns `WouldBlock`
/// - Call `flush_write_buf()` when `Write::write()` returns `WouldBlock`
///
/// # Note on flush()
///
/// The `Write::flush()` method intentionally returns `Ok(())` without checking
/// if there's buffered data. This is for compatibility with libraries like
/// tungstenite that call `flush()` after every write. Actual flushing happens
/// via the async `flush_write_buf()` method.
#[derive(Debug)]
pub struct SyncStream<S> {
    inner: S,
    read_buf: SyncReadBuf,
    write_buf: SyncWriteBuf,
}

/// Read half of a [`SyncStream`] after splitting.
#[derive(Debug)]
pub struct SyncStreamReadHalf<S> {
    inner: S,
    read_buf: SyncReadBuf,
}

/// Write half of a [`SyncStream`] after splitting.
#[derive(Debug)]
pub struct SyncStreamWriteHalf<S> {
    inner: S,
    write_buf: SyncWriteBuf,
}

impl<S> SyncStream<S> {
    /// Creates a new `SyncStream` with default buffer sizes.
    ///
    /// - Base capacity: 8KiB
    /// - Max buffer size: 64MiB
    pub fn new(stream: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, stream)
    }

    /// Creates a new `SyncStream` with a custom base capacity.
    ///
    /// The maximum buffer size defaults to 64MiB.
    pub fn with_capacity(base_capacity: usize, stream: S) -> Self {
        Self::with_limits(base_capacity, DEFAULT_MAX_BUFFER, stream)
    }

    /// Creates a new `SyncStream` with custom base capacity and maximum
    /// buffer size.
    pub fn with_limits(base_capacity: usize, max_buffer_size: usize, stream: S) -> Self {
        Self {
            inner: stream,
            read_buf: SyncReadBuf::new(base_capacity, base_capacity, max_buffer_size),
            write_buf: SyncWriteBuf::new(base_capacity, base_capacity, max_buffer_size),
        }
    }

    pub(crate) fn with_limits2(
        read_capacity: usize,
        write_capacity: usize,
        base_capacity: usize,
        max_buffer_size: usize,
        stream: S,
    ) -> Self {
        Self {
            inner: stream,
            read_buf: SyncReadBuf::new(read_capacity, base_capacity, max_buffer_size),
            write_buf: SyncWriteBuf::new(write_capacity, base_capacity, max_buffer_size),
        }
    }

    /// Returns a reference to the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Consumes the `SyncStream`, returning the underlying stream.
    ///
    /// Any buffered data is discarded. Use [`into_parts`](Self::into_parts)
    /// if you need to preserve unread data.
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Consumes the `SyncStream`, returning the underlying stream and any
    /// unread buffered data.
    ///
    /// If the read buffer is currently lent to an IO operation, the returned
    /// `Vec` will be empty.
    pub fn into_parts(self) -> (S, Vec<u8>) {
        let remaining = self.read_buf.into_inner();
        (self.inner, remaining)
    }

    /// Returns `true` if the stream has reached EOF.
    pub fn is_eof(&self) -> bool {
        self.read_buf.is_eof()
    }

    /// Pull some bytes from this source into the specified buffer.
    pub fn read_buf_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        self.read_buf.read_buf_uninit(buf)
    }

    /// Returns `true` if there is pending data in the write buffer that needs
    /// to be flushed.
    pub fn has_pending_write(&self) -> bool {
        self.write_buf.has_pending_write()
    }
}

impl<S> SyncStreamReadHalf<S> {
    /// Returns `true` if the stream has reached EOF.
    pub fn is_eof(&self) -> bool {
        self.read_buf.is_eof()
    }

    /// Pull some bytes from this source into the specified buffer.
    pub fn read_buf_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        self.read_buf.read_buf_uninit(buf)
    }
}

impl<S> SyncStreamWriteHalf<S> {
    /// Returns `true` if there is pending data in the write buffer that needs
    /// to be flushed.
    pub fn has_pending_write(&self) -> bool {
        self.write_buf.has_pending_write()
    }
}

impl<S> Read for SyncStream<S> {
    /// Reads data from the internal buffer.
    ///
    /// Returns `WouldBlock` if the buffer is empty and not at EOF,
    /// indicating that `fill_read_buf()` should be called.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_buf.read(buf)
    }

    #[cfg(feature = "read_buf")]
    fn read_buf(&mut self, buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        self.read_buf.read_buf(buf)
    }
}

impl<S> BufRead for SyncStream<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.read_buf.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.read_buf.consume(amt);
    }
}

impl<S> Read for SyncStreamReadHalf<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_buf.read(buf)
    }

    #[cfg(feature = "read_buf")]
    fn read_buf(&mut self, buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        self.read_buf.read_buf(buf)
    }
}

impl<S> BufRead for SyncStreamReadHalf<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.read_buf.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.read_buf.consume(amt);
    }
}

impl<S> Write for SyncStream<S> {
    /// Writes data to the internal buffer.
    ///
    /// Returns `WouldBlock` if the buffer needs flushing or has reached max
    /// capacity. In the latter case, it may write partial data before
    /// returning `WouldBlock`.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_buf.write(buf)
    }

    /// Returns `Ok(())` without checking for buffered data.
    ///
    /// **Important**: This does NOT actually flush data to the underlying
    /// stream. This behavior is intentional for compatibility with
    /// libraries like tungstenite that call `flush()` after every write
    /// operation. The actual async flush happens when `flush_write_buf()`
    /// is called.
    ///
    /// This prevents spurious errors in sync code that expects `flush()` to
    /// succeed after successfully buffering data.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<S> Write for SyncStreamWriteHalf<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_buf.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn would_block(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, msg)
}

impl<S: crate::AsyncRead> SyncStream<S> {
    /// Fills the read buffer by reading from the underlying async stream.
    ///
    /// This method:
    /// 1. Compacts the buffer if there's unconsumed data
    /// 2. Ensures there's space for at least `base_capacity` more bytes
    /// 3. Reads data from the underlying stream
    /// 4. Returns the number of bytes read (0 indicates EOF)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The read buffer has reached `max_buffer_size`
    /// - The underlying stream returns an error
    pub async fn fill_read_buf(&mut self) -> io::Result<usize> {
        self.read_buf.fill_read_buf(&mut self.inner).await
    }
}

impl<S: crate::AsyncRead> SyncStreamReadHalf<S> {
    /// See [`SyncStream::fill_read_buf`].
    pub async fn fill_read_buf(&mut self) -> io::Result<usize> {
        self.read_buf.fill_read_buf(&mut self.inner).await
    }
}

impl<S: crate::AsyncWrite> SyncStream<S> {
    /// Flushes the write buffer to the underlying async stream.
    ///
    /// This method:
    /// 1. Writes all buffered data to the underlying stream
    /// 2. Calls `flush()` on the underlying stream
    /// 3. Returns the total number of bytes flushed
    ///
    /// On error, any unwritten data remains in the buffer and can be retried.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying stream returns an error.
    /// In this case, the buffer retains any data that wasn't successfully
    /// written.
    pub async fn flush_write_buf(&mut self) -> io::Result<usize> {
        self.write_buf.flush_write_buf(&mut self.inner).await
    }
}

impl<S: crate::AsyncWrite> SyncStreamWriteHalf<S> {
    /// See [`SyncStream::flush_write_buf`].
    pub async fn flush_write_buf(&mut self) -> io::Result<usize> {
        self.write_buf.flush_write_buf(&mut self.inner).await
    }
}

impl<S: Splittable> Splittable for SyncStream<S> {
    type ReadHalf = SyncStreamReadHalf<S::ReadHalf>;
    type WriteHalf = SyncStreamWriteHalf<S::WriteHalf>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        let (r, w) = self.inner.split();
        let read_half = SyncStreamReadHalf {
            inner: r,
            read_buf: self.read_buf,
        };
        let write_half = SyncStreamWriteHalf {
            inner: w,
            write_buf: self.write_buf,
        };
        (read_half, write_half)
    }
}
