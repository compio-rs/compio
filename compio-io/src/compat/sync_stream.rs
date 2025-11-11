use std::{
    io::{self, BufRead, Read, Write},
    mem::MaybeUninit,
};

use compio_buf::{BufResult, IntoInner, IoBuf};

use crate::util::DEFAULT_BUF_SIZE;

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
    read_buf: Vec<u8>,
    read_pos: usize,
    write_buf: Vec<u8>,
    eof: bool,
    base_capacity: usize,
    max_buffer_size: usize,
}

impl<S> SyncStream<S> {
    // 8KB base
    const DEFAULT_MAX_BUFFER: usize = 64 * 1024 * 1024;

    // 64MB max

    /// Creates a new `SyncStream` with default buffer sizes.
    ///
    /// - Base capacity: 8KB
    /// - Max buffer size: 64MB
    pub fn new(stream: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, stream)
    }

    /// Creates a new `SyncStream` with a custom base capacity.
    ///
    /// The maximum buffer size defaults to 64MB.
    pub fn with_capacity(base_capacity: usize, stream: S) -> Self {
        Self {
            inner: stream,
            read_buf: Vec::with_capacity(base_capacity),
            read_pos: 0,
            write_buf: Vec::with_capacity(base_capacity),
            eof: false,
            base_capacity,
            max_buffer_size: Self::DEFAULT_MAX_BUFFER,
        }
    }

    /// Creates a new `SyncStream` with custom base capacity and maximum
    /// buffer size.
    pub fn with_limits(base_capacity: usize, max_buffer_size: usize, stream: S) -> Self {
        Self {
            inner: stream,
            read_buf: Vec::with_capacity(base_capacity),
            read_pos: 0,
            write_buf: Vec::with_capacity(base_capacity),
            eof: false,
            base_capacity,
            max_buffer_size,
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
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Returns `true` if the stream has reached EOF.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Returns the available bytes in the read buffer.
    fn available_read(&self) -> &[u8] {
        &self.read_buf[self.read_pos..]
    }

    /// Marks `amt` bytes as consumed from the read buffer.
    ///
    /// Resets the buffer when all data is consumed and shrinks capacity
    /// if it has grown significantly beyond the base capacity.
    fn consume_read(&mut self, amt: usize) {
        self.read_pos += amt;

        // Shrink oversized buffers back to base capacity
        if self.read_pos >= self.read_buf.len() {
            self.read_pos = 0;

            if self.read_buf.capacity() > self.base_capacity * 4 {
                self.read_buf = Vec::with_capacity(self.base_capacity);
            } else {
                self.read_buf.clear();
            }
        }
    }

    /// Pull some bytes from this source into the specified buffer.
    pub fn read_buf_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        let available = self.fill_buf()?;

        let to_read = available.len().min(buf.len());
        buf[..to_read].copy_from_slice(unsafe {
            std::slice::from_raw_parts(available.as_ptr().cast(), to_read)
        });
        self.consume(to_read);

        Ok(to_read)
    }
}

impl<S> Read for SyncStream<S> {
    /// Reads data from the internal buffer.
    ///
    /// Returns `WouldBlock` if the buffer is empty and not at EOF,
    /// indicating that `fill_read_buf()` should be called.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut slice = self.fill_buf()?;
        slice.read(buf).inspect(|res| {
            self.consume(*res);
        })
    }

    #[cfg(feature = "read_buf")]
    fn read_buf(&mut self, mut buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        let mut slice = self.fill_buf()?;
        let old_written = buf.written();
        slice.read_buf(buf.reborrow())?;
        let len = buf.written() - old_written;
        self.consume(len);
        Ok(())
    }
}

impl<S> BufRead for SyncStream<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let available = self.available_read();

        if available.is_empty() && !self.eof {
            return Err(would_block("need to fill read buffer"));
        }

        Ok(available)
    }

    fn consume(&mut self, amt: usize) {
        self.consume_read(amt);
    }
}

impl<S> Write for SyncStream<S> {
    /// Writes data to the internal buffer.
    ///
    /// Returns `WouldBlock` if the buffer needs flushing or has reached max
    /// capacity. In the latter case, it may write partial data before
    /// returning `WouldBlock`.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Check if we should flush first
        if self.write_buf.len() > self.base_capacity * 2 / 3 && !self.write_buf.is_empty() {
            return Err(would_block("need to flush write buffer"));
        }

        // Check if write would exceed max buffer size
        if self.write_buf.len() + buf.len() > self.max_buffer_size {
            let space = self.max_buffer_size - self.write_buf.len();
            if space == 0 {
                return Err(would_block("write buffer full, need to flush"));
            }
            self.write_buf.extend_from_slice(&buf[..space]);
            return Ok(space);
        }

        self.write_buf.extend_from_slice(buf);
        Ok(buf.len())
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
        if self.eof {
            return Ok(0);
        }

        // Compact buffer, move unconsumed data to the front
        if self.read_pos > 0 && self.read_pos < self.read_buf.len() {
            let buf_len = self.read_buf.len();
            let remaining = buf_len - self.read_pos;
            self.read_buf.copy_within(self.read_pos..buf_len, 0);

            // SAFETY: We're setting the length to the amount of data we just moved.
            // The data from 0..remaining is initialized (just moved from read_pos..buf_len)
            unsafe {
                self.read_buf.set_len(remaining);
            }
            self.read_pos = 0;
        } else if self.read_pos >= self.read_buf.len() {
            // All data consumed, reset buffer
            self.read_pos = 0;
            if self.read_buf.capacity() > self.base_capacity * 4 {
                self.read_buf = Vec::with_capacity(self.base_capacity);
            } else {
                self.read_buf.clear();
            }
        }

        let current_len = self.read_buf.len();

        if current_len >= self.max_buffer_size {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!("read buffer size limit ({}) exceeded", self.max_buffer_size),
            ));
        }

        let capacity = self.read_buf.capacity();
        let available_space = capacity - current_len;

        let target_space = self.base_capacity;
        if available_space < target_space {
            let new_capacity = current_len + target_space;
            self.read_buf.reserve_exact(new_capacity - capacity);
        }

        let capacity = self.read_buf.capacity();
        let len = self.read_buf.len();

        // SAFETY: We're extending the buffer to its capacity to allow reading into
        // uninitialized memory. This is safe because:
        // 1. We save the original length and restore it on error
        // 2. The async read operation initializes the bytes it writes to
        // 3. We update the length based on how many bytes were actually read
        unsafe {
            self.read_buf.set_len(capacity);
        }

        let buf = std::mem::take(&mut self.read_buf);

        let read_slice = IoBuf::slice(buf, len..);

        let BufResult(result, mut buf) = self.inner.read(read_slice).await.into_inner();

        match result {
            Ok(n) => {
                if n == 0 {
                    self.eof = true;
                    unsafe {
                        buf.set_len(len);
                    }
                } else {
                    unsafe {
                        buf.set_len(len + n);
                    }
                }
                self.read_buf = buf;
                Ok(n)
            }
            Err(e) => {
                unsafe {
                    buf.set_len(len);
                }
                self.read_buf = buf;
                Err(e)
            }
        }
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
        if self.write_buf.is_empty() {
            return Ok(0);
        }

        let total = self.write_buf.len();
        let mut buf = std::mem::take(&mut self.write_buf);
        let mut flushed = 0;

        while flushed < total {
            let write_slice = IoBuf::slice(buf, flushed..);

            let BufResult(result, returned_buf) = self.inner.write(write_slice).await.into_inner();
            buf = returned_buf;

            match result {
                Ok(0) => {
                    self.write_buf = buf[flushed..].to_vec();
                    return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
                }
                Ok(n) => {
                    flushed += n;
                }
                Err(e) => {
                    self.write_buf = buf[flushed..].to_vec();
                    return Err(e);
                }
            }
        }

        self.write_buf = Vec::with_capacity(self.base_capacity);

        self.inner.flush().await?;

        Ok(flushed)
    }
}
