//! The io-uring buffer pool. It is backed by a [`Vec`] of [`Vec<u8>`].
//! The kernel selects the buffer and returns `flags`. The crate
//! [`io_uring_buf_ring`] handles the returning of buffer on drop.

use std::{
    borrow::{Borrow, BorrowMut},
    fmt::{Debug, Formatter},
    io,
    ops::{Deref, DerefMut},
};

use io_uring_buf_ring::IoUringBufRing;

/// Buffer pool
///
/// A buffer pool to allow user no need to specify a specific buffer to do the
/// IO operation
pub struct BufferPool {
    buf_ring: IoUringBufRing<Vec<u8>>,
}

impl Debug for BufferPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish_non_exhaustive()
    }
}

impl BufferPool {
    pub(crate) fn new(buf_ring: IoUringBufRing<Vec<u8>>) -> Self {
        Self { buf_ring }
    }

    pub(crate) fn buffer_group(&self) -> u16 {
        self.buf_ring.buffer_group()
    }

    pub(crate) fn into_inner(self) -> IoUringBufRing<Vec<u8>> {
        self.buf_ring
    }

    /// ## Safety
    /// * `available_len` should be the returned value from the op.
    pub(crate) unsafe fn get_buffer(
        &self,
        buffer_id: u16,
        available_len: usize,
    ) -> io::Result<BorrowedBuffer<'_>> {
        unsafe { self.buf_ring.get_buf(buffer_id, available_len) }
            .map(BorrowedBuffer)
            .ok_or_else(|| io::Error::other(format!("cannot find buffer {buffer_id}")))
    }

    pub(crate) fn reuse_buffer(&self, buffer_id: u16) {
        // SAFETY: 0 is always valid length. We just want to get the buffer once and
        // return it immediately.
        unsafe { self.buf_ring.get_buf(buffer_id, 0) };
    }
}

/// Buffer borrowed from buffer pool
///
/// When IO operation finish, user will obtain a `BorrowedBuffer` to access the
/// filled data
pub struct BorrowedBuffer<'a>(io_uring_buf_ring::BorrowedBuffer<'a, Vec<u8>>);

impl Debug for BorrowedBuffer<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorrowedBuffer").finish_non_exhaustive()
    }
}

impl Deref for BorrowedBuffer<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DerefMut for BorrowedBuffer<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl AsRef<[u8]> for BorrowedBuffer<'_> {
    fn as_ref(&self) -> &[u8] {
        self.deref()
    }
}

impl AsMut<[u8]> for BorrowedBuffer<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.deref_mut()
    }
}

impl Borrow<[u8]> for BorrowedBuffer<'_> {
    fn borrow(&self) -> &[u8] {
        self.deref()
    }
}

impl BorrowMut<[u8]> for BorrowedBuffer<'_> {
    fn borrow_mut(&mut self) -> &mut [u8] {
        self.deref_mut()
    }
}
