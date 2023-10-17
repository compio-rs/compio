//! A utility buffer type to implement [`BufReader`] and [`BufWriter`]
//!
//! [`BufReader`]: crate::read::BufReader
//! [`BufWriter`]: crate::write::BufWriter
use std::{
    fmt::{self, Debug},
    future::Future,
};

use compio_buf::{BufResult, IoBuf, IoBufMut, SetBufInit};

use crate::util::MISSING_BUF;

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
        self.buf.set_len(len);
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
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self(Some(Inner {
            buf: Vec::with_capacity(cap),
            pos: 0,
        }))
    }

    #[inline]
    pub fn slice(&self) -> &[u8] {
        self.inner().slice()
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
    pub fn need_flush(&self) -> bool {
        // TODO: Better way to determine if we need to flush the buffer
        let buf = self.buf();
        buf.len() > buf.capacity() * 2 / 3
    }

    #[inline]
    pub fn reset(&mut self) {
        self.inner_mut().reset();
    }

    /// Execute a funcition with ownership of the buffer, and restore the buffer
    /// afterwards
    pub async fn with<R, Fut, F>(&mut self, func: F) -> std::io::Result<R>
    where
        Fut: Future<Output = BufResult<R, Inner>>,
        F: FnOnce(Inner) -> Fut,
    {
        let BufResult(res, buf) = func(self.take_inner()).await;
        self.restore_inner(buf);
        res
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
