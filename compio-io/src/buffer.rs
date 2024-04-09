//! A utility buffer type to implement [`BufReader`] and [`BufWriter`]
//!
//! [`BufReader`]: crate::read::BufReader
//! [`BufWriter`]: crate::write::BufWriter
use std::{
    fmt::{self, Debug},
    future::Future,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit, Slice};

use crate::{util::MISSING_BUF, AsyncWrite, IoResult};

pub struct Inner {
    buf: Vec<u8>,
    pos: usize,
}

impl Inner {
    #[inline]
    fn all_done(&self) -> bool {
        self.buf.len() == self.pos
    }

    /// Move pos & init needle to 0
    #[inline]
    fn reset(&mut self) {
        self.pos = 0;
        unsafe { self.buf.set_len(0) };
    }

    #[inline]
    fn slice(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    #[inline]
    pub(crate) fn into_slice(self) -> Slice<Self> {
        let pos = self.pos;
        IoBuf::slice(self, pos..)
    }
}

unsafe impl IoBuf for Inner {
    #[inline]
    fn as_buf_ptr(&self) -> *const u8 {
        self.buf.as_ptr()
    }

    #[inline]
    fn buf_len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    fn buf_capacity(&self) -> usize {
        self.buf.capacity()
    }
}

impl SetBufInit for Inner {
    #[inline]
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.buf.set_buf_init(len);
    }
}

unsafe impl IoBufMut for Inner {
    #[inline]
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.buf.as_mut_ptr()
    }
}

/// A buffer with an internal progress tracker
///
/// ```plain
/// +------------------------------------------------+
/// |               Buf: Vec<u8> cap                 |
/// +--------------------+-------------------+-------+
/// +-- Progress (pos) --^                   |
/// +-------- Initialized (vec len) ---------^
///                      +------ slice ------^
/// ```
pub struct Buffer(Option<Inner>);

impl Buffer {
    /// Create a buffer with capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self(Some(Inner {
            buf: Vec::with_capacity(cap),
            pos: 0,
        }))
    }

    /// Get the initialized but not consumed part of the buffer.
    #[inline]
    pub fn slice(&self) -> &[u8] {
        self.inner().slice()
    }

    /// If the inner buffer is empty.
    #[inline]
    #[allow(unused)]
    pub fn is_empty(&self) -> bool {
        self.inner().as_slice().is_empty()
    }

    /// All bytes in the buffer have been read
    #[inline]
    pub fn all_done(&self) -> bool {
        self.inner().all_done()
    }

    /// The buffer needs to be filled
    #[inline]
    pub fn need_fill(&self) -> bool {
        // TODO: Better way to determine if we need to fill the buffer
        let buf = self.buf();
        buf.len() < buf.capacity() / 3
    }

    /// The buffer needs to be flushed
    #[inline]
    pub fn need_flush(&self) -> bool {
        // TODO: Better way to determine if we need to flush the buffer
        let buf = self.buf();
        buf.len() > buf.capacity() * 2 / 3
    }

    /// Clear the inner buffer and reset the position to the start.
    #[inline]
    pub fn reset(&mut self) {
        self.inner_mut().reset();
    }

    /// Execute a funcition with ownership of the buffer, and restore the buffer
    /// afterwards
    pub async fn with<R, Fut, F>(&mut self, func: F) -> IoResult<R>
    where
        Fut: Future<Output = BufResult<R, Inner>>,
        F: FnOnce(Inner) -> Fut,
    {
        let BufResult(res, buf) = func(self.take_inner()).await;
        self.restore_inner(buf);
        res
    }

    /// Execute a funcition with ownership of the buffer, and restore the buffer
    /// afterwards
    pub fn with_sync<R>(
        &mut self,
        func: impl FnOnce(Inner) -> BufResult<R, Inner>,
    ) -> std::io::Result<R> {
        let BufResult(res, buf) = func(self.take_inner());
        self.restore_inner(buf);
        res
    }

    /// Wrapper to flush the buffer to a writer with error safety.
    ///
    /// https://github.com/compio-rs/compio/issues/209
    pub async fn flush_to(&mut self, writer: &mut impl AsyncWrite) -> IoResult<usize> {
        if self.slice().is_empty() {
            return Ok(0);
        }
        let mut total = 0;
        loop {
            let written = self
                .with(|inner| async { writer.write(inner.into_slice()).await.into_inner() })
                .await?;
            if written == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "cannot flush all buffer data",
                ));
            }
            total += written;
            if self.advance(written) {
                break;
            }
        }
        self.reset();
        Ok(total)
    }

    /// Mark some bytes as read by advancing the progress tracker, return a
    /// `bool` indicating if all bytes are read.
    #[inline]
    pub fn advance(&mut self, amount: usize) -> bool {
        debug_assert!(self.inner().pos + amount <= self.inner().buf_capacity());

        let inner = self.inner_mut();
        inner.pos += amount;
        inner.all_done()
    }

    #[inline]
    fn take_inner(&mut self) -> Inner {
        self.0.take().expect(MISSING_BUF)
    }

    #[inline]
    fn restore_inner(&mut self, buf: Inner) {
        debug_assert!(self.0.is_none());

        self.0 = Some(buf);
    }

    #[inline]
    fn inner(&self) -> &Inner {
        self.0.as_ref().expect(MISSING_BUF)
    }

    #[inline]
    fn inner_mut(&mut self) -> &mut Inner {
        self.0.as_mut().expect(MISSING_BUF)
    }

    #[inline]
    fn buf(&self) -> &Vec<u8> {
        &self.inner().buf
    }
}

impl Debug for Buffer {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner();
        fmt.debug_struct("Buffer")
            .field("capacity", &inner.buf_capacity())
            .field("init", &inner.buf_len())
            .field("progress", &inner.pos)
            .finish()
    }
}
