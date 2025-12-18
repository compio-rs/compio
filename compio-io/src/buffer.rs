//! A utility buffer type to implement [`BufReader`] and [`BufWriter`]
//!
//! [`BufReader`]: crate::read::BufReader
//! [`BufWriter`]: crate::write::BufWriter
use std::{
    fmt::{self, Debug},
    future::Future,
};

use compio_buf::{
    BufResult, IntoInner, IoBuf, IoBufMut, ReserveError, ReserveExactError, SetLen, Slice,
};

use crate::{AsyncWrite, IoResult, util::MISSING_BUF};

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
        self.buf.clear();
    }

    #[inline]
    fn slice(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    pub fn reserve_exact(&mut self, additional: usize) {
        self.buf.reserve_exact(additional);
    }

    pub fn extend_from_slice(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn compact_to(&mut self, capacity: usize, max_capacity: usize) {
        if self.pos > 0 && self.pos < self.buf.len() {
            let buf_len = self.buf.len();
            let remaining = buf_len - self.pos;
            self.buf.copy_within(self.pos..buf_len, 0);

            // SAFETY: We're setting the length to the amount of data we just moved.
            // The data from 0..remaining is initialized (just moved from read_pos..buf_len)
            unsafe {
                self.buf.set_len(remaining);
            }
            self.pos = 0;
        } else if self.pos >= self.buf.len() {
            // All data consumed, reset buffer
            self.reset();
            if self.buf.capacity() > max_capacity {
                self.buf.shrink_to(capacity);
            }
        }
    }

    #[inline]
    pub(crate) fn into_slice(self) -> Slice<Self> {
        let pos = self.pos;
        self.slice(pos..)
    }
}

impl IoBuf for Inner {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        self.buf.as_slice()
    }
}

impl SetLen for Inner {
    #[inline]
    unsafe fn set_len(&mut self, len: usize) {
        unsafe { self.buf.set_len(len) }
    }
}

impl IoBufMut for Inner {
    #[inline]
    fn as_uninit(&mut self) -> &mut [std::mem::MaybeUninit<u8>] {
        self.buf.as_uninit()
    }

    fn reserve(&mut self, len: usize) -> Result<(), ReserveError> {
        IoBufMut::reserve(&mut self.buf, len)
    }

    fn reserve_exact(&mut self, len: usize) -> Result<(), ReserveExactError> {
        IoBufMut::reserve_exact(&mut self.buf, len)
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
    pub fn buffer(&self) -> &[u8] {
        self.inner().slice()
    }

    /// If the inner buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner().slice().is_empty()
    }

    /// All bytes in the buffer have been read
    #[inline]
    pub fn all_done(&self) -> bool {
        self.inner().all_done()
    }

    /// The buffer needs to be filled
    #[inline]
    pub fn need_fill(&self) -> bool {
        self.is_empty()
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

    /// Reserve additional capacity in the buffer. See detail at
    /// [`Vec::reserve`].
    pub fn reserve(&mut self, additional: usize) {
        self.inner_mut().buf.reserve(additional);
    }

    /// Compact the buffer to the given capacity, if the current capacity is
    /// larger than the given maximum capacity.
    pub fn compact_to(&mut self, capacity: usize, max_capacity: usize) {
        self.inner_mut().compact_to(capacity, max_capacity);
    }

    /// Execute a funcition with ownership of the buffer, and restore the buffer
    /// afterwards
    pub async fn with<R, Fut, F>(&mut self, func: F) -> IoResult<R>
    where
        F: FnOnce(Inner) -> Fut,
        Fut: Future<Output = BufResult<R, Inner>>,
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
        if self.inner().slice().is_empty() {
            return Ok(0);
        }
        let mut total = 0;
        loop {
            let written = self
                .with(|inner| async { writer.write(inner.into_slice()).await.into_inner() })
                .await?;
            if written == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
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
        debug_assert!(self.inner().pos + amount <= self.inner().buf.capacity());

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
            .field("capacity", &inner.buf.capacity())
            .field("init", &inner.buf.len())
            .field("progress", &inner.pos)
            .finish()
    }
}
