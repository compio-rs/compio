//! A utility buffer type to implement [`BufReader`] and [`BufWriter`]
//!
//! [`BufReader`]: crate::read::BufReader
//! [`BufWriter`]: crate::write::BufWriter
use std::{
    fmt::{self, Debug},
    future::Future,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, ReserveError, Slice};

use crate::{AsyncWrite, IoResult, util::MISSING_BUF};

/// A buffer with an internal progress tracker
///
/// ```plain
/// +---------------------------------------------------------+
/// |                   Buf: IoBufMut cap                     |
/// +----------------------------+-------------------+--------+
/// +-- Progress (slice.begin) --^                   |
/// +-------------- Initialized (len) ---------------^
///                              +------ slice ------^
/// ```
pub struct Buffer<B = Vec<u8>>(Option<Slice<B>>);

impl Buffer<Vec<u8>> {
    pub fn new() -> Self {
        Self(Some(Vec::new().slice(..)))
    }

    /// Create a buffer with capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self(Some(Vec::with_capacity(cap).slice(..)))
    }

    /// Compact the buffer to the given capacity, if the current capacity is
    /// larger than the given maximum capacity.
    #[allow(dead_code)]
    pub fn compact_to(&mut self, capacity: usize, max_capacity: usize) {
        let inner = self.take_inner();
        let pos = inner.begin();
        let mut buf = inner.into_inner();

        if pos > 0 && pos < buf.len() {
            // Within the buffer, still has remaining data, move those to front
            let buf_len = buf.len();
            let remaining = buf_len - pos;
            buf.copy_within(pos..buf_len, 0);

            // SAFETY: We're setting the length to the amount of data we just moved.
            // The data from 0..remaining is initialized (just moved from read_pos..buf_len)
            unsafe {
                buf.set_len(remaining);
            }
        } else if pos >= buf.len() {
            // All data consumed, reset buffer
            buf.clear();
            if buf.capacity() > max_capacity {
                buf.shrink_to(capacity);
            }
        }

        self.restore_inner(buf.slice(..));
    }
}

impl<B> Buffer<B> {
    #[inline]
    pub(crate) fn take_inner(&mut self) -> Slice<B> {
        self.0.take().expect(MISSING_BUF)
    }

    #[inline]
    pub(crate) fn restore_inner(&mut self, buf: Slice<B>) {
        debug_assert!(self.0.is_none());

        self.0 = Some(buf);
    }

    #[inline]
    pub(crate) fn inner(&self) -> &Slice<B> {
        self.0.as_ref().expect(MISSING_BUF)
    }

    #[inline]
    fn inner_mut(&mut self) -> &mut Slice<B> {
        self.0.as_mut().expect(MISSING_BUF)
    }

    #[inline]
    fn buf(&self) -> &B {
        self.inner().as_inner()
    }

    #[inline]
    fn buf_mut(&mut self) -> &mut B {
        self.inner_mut().as_inner_mut()
    }

    pub(crate) fn has_inner(&self) -> bool {
        self.0.is_some()
    }
}

impl<B: IoBufMut> Buffer<B> {
    pub fn new_with(buf: B) -> Self {
        Self(Some(buf.slice(..)))
    }

    /// Get the initialized but not consumed part of the buffer.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        self.inner()
    }

    /// If the underlying buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf().is_empty()
    }

    /// All bytes in the buffer have been read
    #[inline]
    pub fn all_done(&self) -> bool {
        // sliced view is empty meaning all bytes have been read
        self.inner().is_empty()
    }

    /// The buffer needs to be filled
    #[inline]
    pub fn need_fill(&self) -> bool {
        self.is_empty()
    }

    /// The buffer needs to be flushed
    #[inline]
    pub fn need_flush(&mut self) -> bool {
        // TODO: Better way to determine if we need to flush the buffer
        let buf = self.buf_mut();
        let cap = buf.buf_capacity();
        let len = (*buf).buf_len();
        len > cap * 2 / 3
    }

    /// Clear the inner buffer and reset the position to the start.
    #[inline]
    pub fn reset(&mut self) {
        let mut buf = self.take_inner().into_inner();
        buf.clear();
        self.restore_inner(buf.slice(..));
    }

    /// Reserve additional capacity in the buffer.
    ///
    /// # Panics
    ///
    /// Panics if reserving additional capacity fails (most likely OOM).
    pub fn reserve(&mut self, additional: usize) {
        if let Err(ReserveError::ReserveFailed(e)) = self.try_reserve(additional) {
            panic!("Buffer reserve failed: {}", e)
        }
    }

    /// Try reserve additional capacity in the buffer. See detail at
    /// [`IoBufMut::reserve`].
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), ReserveError> {
        self.inner_mut().reserve(additional)
    }

    /// Execute a funcition with ownership of the buffer, and restore the buffer
    /// afterwards
    pub async fn with<R, Fut, F>(&mut self, func: F) -> IoResult<R>
    where
        F: FnOnce(Slice<B>) -> Fut,
        Fut: Future<Output = BufResult<R, Slice<B>>>,
    {
        let BufResult(res, buf) = func(self.take_inner()).await;
        self.restore_inner(buf);
        res
    }

    /// Execute a funcition with ownership of the buffer, and restore the buffer
    /// afterwards
    pub fn with_sync<R>(
        &mut self,
        func: impl FnOnce(Slice<B>) -> BufResult<R, Slice<B>>,
    ) -> std::io::Result<R> {
        let BufResult(res, buf) = func(self.take_inner());
        self.restore_inner(buf);
        res
    }

    /// Wrapper to flush the buffer to a writer with error safety.
    ///
    /// https://github.com/compio-rs/compio/issues/209
    pub async fn flush_to(&mut self, writer: &mut impl AsyncWrite) -> IoResult<usize> {
        if self.inner().is_empty() {
            return Ok(0);
        }
        let mut total = 0;
        loop {
            let written = self.with(|inner| writer.write(inner)).await?;
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
        assert!(self.inner().begin() + amount <= self.buf_mut().buf_capacity());

        let inner = self.take_inner();
        let pos = inner.begin() + amount;

        self.restore_inner(inner.into_inner().slice(pos..));
        self.all_done()
    }
}

impl<B: IoBuf> Debug for Buffer<B> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Buffer")
            .field("capacity", &"...") // `buf_capacity()` requires `&mut self`
            .field("init", &self.buf().buf_len())
            .field("progress", &self.inner().begin())
            .finish()
    }
}
