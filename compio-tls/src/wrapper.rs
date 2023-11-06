use std::io::{self, BufRead, Read, Write};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit};
use compio_io::{AsyncWriteExt, Buffer};

const DEFAULT_BUF_SIZE: usize = 8 * 1024;

#[derive(Debug)]
pub struct StreamWrapper<S> {
    stream: S,
    eof: bool,
    read_buffer: Buffer,
    write_buffer: Buffer,
}

impl<S> StreamWrapper<S> {
    pub fn new(stream: S) -> Self {
        Self::with_capacity(stream, DEFAULT_BUF_SIZE)
    }

    pub fn with_capacity(stream: S, cap: usize) -> Self {
        Self {
            stream,
            eof: false,
            read_buffer: Buffer::with_capacity(cap),
            write_buffer: Buffer::with_capacity(cap),
        }
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    pub fn get_ref(&self) -> &S {
        &self.stream
    }

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

impl<S> Read for StreamWrapper<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut slice = self.fill_buf()?;
        slice.read(buf).map(|res| {
            self.consume(res);
            res
        })
    }
}

impl<S> BufRead for StreamWrapper<S> {
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

impl<S> Write for StreamWrapper<S> {
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

impl<S: compio_io::AsyncRead> StreamWrapper<S> {
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

impl<S: compio_io::AsyncWrite> StreamWrapper<S> {
    pub async fn flush_write_buf(&mut self) -> io::Result<usize> {
        let stream = &mut self.stream;
        let len = self.write_buffer.with(|b| stream.write_all(b)).await?;
        self.write_buffer.reset();
        stream.flush().await?;
        Ok(len)
    }
}
