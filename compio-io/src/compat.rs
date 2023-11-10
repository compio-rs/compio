//! Compat wrappers for interop with other crates.

use std::io::{self, BufRead, Read, Write};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit};

use crate::{buffer::Buffer, util::DEFAULT_BUF_SIZE, AsyncWriteExt};

/// A wrapper for [`AsyncRead`](crate::AsyncRead) +
/// [`AsyncWrite`](crate::AsyncWrite), providing sync traits impl. The sync
/// methods will return [`io::ErrorKind::WouldBlock`] error if the inner buffer
/// needs more data.
#[derive(Debug)]
pub struct SyncStream<S> {
    stream: S,
    eof: bool,
    read_buffer: Buffer,
    write_buffer: Buffer,
}

impl<S> SyncStream<S> {
    /// Create [`SyncStream`] with the stream and default buffer size.
    pub fn new(stream: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, stream)
    }

    /// Create [`SyncStream`] with the stream and buffer size.
    pub fn with_capacity(cap: usize, stream: S) -> Self {
        Self {
            stream,
            eof: false,
            read_buffer: Buffer::with_capacity(cap),
            write_buffer: Buffer::with_capacity(cap),
        }
    }

    /// Get if the stream is at EOF.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Get the reference of the inner stream.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    /// Get the mutable reference of the inner stream.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    fn flush_impl(&mut self) -> io::Result<()> {
        if !self.write_buffer.is_empty() {
            Err(would_block("need to flush the write buffer"))
        } else {
            Ok(())
        }
    }
}

impl<S> Read for SyncStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut slice = self.fill_buf()?;
        slice.read(buf).map(|res| {
            self.consume(res);
            res
        })
    }
}

impl<S> BufRead for SyncStream<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.read_buffer.all_done() {
            self.read_buffer.reset();
        }

        if self.read_buffer.slice().is_empty() && !self.eof {
            return Err(would_block("need to fill the read buffer"));
        }

        Ok(self.read_buffer.slice())
    }

    fn consume(&mut self, amt: usize) {
        self.read_buffer.advance(amt);
    }
}

impl<S> Write for SyncStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.write_buffer.need_flush() {
            self.flush_impl()?;
        }

        let written = self.write_buffer.with_sync(|mut inner| {
            let len = buf.len().min(inner.buf_capacity() - inner.buf_len());
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buf.as_ptr(),
                    inner.as_buf_mut_ptr().add(inner.buf_len()),
                    len,
                );
                inner.set_buf_init(inner.buf_len() + len);
            }
            BufResult(Ok(len), inner)
        })?;

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Related PR:
        // https://github.com/sfackler/rust-openssl/pull/1922
        // After this PR merged, we can use self.flush_impl()
        Ok(())
    }
}

fn would_block(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, msg)
}

impl<S: crate::AsyncRead> SyncStream<S> {
    /// Fill the read buffer.
    pub async fn fill_read_buf(&mut self) -> io::Result<usize> {
        let stream = &mut self.stream;
        let len = self
            .read_buffer
            .with(|b| async move {
                let len = b.buf_len();
                let b = b.slice(len..);
                stream.read(b).await.into_inner()
            })
            .await?;
        if len == 0 {
            self.eof = true;
        }
        Ok(len)
    }
}

impl<S: crate::AsyncWrite> SyncStream<S> {
    /// Flush all data in the write buffer.
    pub async fn flush_write_buf(&mut self) -> io::Result<usize> {
        let stream = &mut self.stream;
        let len = self.write_buffer.with(|b| stream.write_all(b)).await?;
        self.write_buffer.reset();
        stream.flush().await?;
        Ok(len)
    }
}
