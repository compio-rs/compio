//! The io-uring buffer pool. It is backed by a [`Vec`] of [`Vec<u8>`].
//! The kernel selects the buffer and returns `flags`. The crate
//! [`io_uring_buf_ring`] handles the returning of buffer on drop.

use std::{
    borrow::{Borrow, BorrowMut},
    fmt::{Debug, Formatter},
    io,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use io_uring_buf_ring::IoUringBufRing;
#[cfg(not(fusion))]
pub use {BufferPool as IoUringBufferPool, OwnedBuffer as IoUringOwnedBuffer};

struct BufferPoolInner {
    buf_ring: IoUringBufRing<Vec<u8>>,
}

impl BufferPoolInner {
    fn reuse_buffer(&self, buffer_id: u16) {
        // SAFETY: 0 is always valid length. We just want to get the buffer once and
        // return it immediately.
        unsafe { self.buf_ring.get_buf(buffer_id, 0) };
    }
}

/// Buffer pool
///
/// A buffer pool to allow user no need to specify a specific buffer to do the
/// IO operation
#[derive(Clone)]
pub struct BufferPool {
    inner: Rc<BufferPoolInner>,
}

impl Debug for BufferPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish_non_exhaustive()
    }
}

impl BufferPool {
    pub(crate) fn new(buf_ring: IoUringBufRing<Vec<u8>>) -> Self {
        Self {
            inner: Rc::new(BufferPoolInner { buf_ring }),
        }
    }

    pub(crate) fn buffer_group(&self) -> u16 {
        self.inner.buf_ring.buffer_group()
    }

    pub(crate) fn into_inner(self) -> Result<IoUringBufRing<Vec<u8>>, Self> {
        Rc::try_unwrap(self.inner)
            .map(|inner| inner.buf_ring)
            .map_err(|inner| Self { inner })
    }

    #[doc(hidden)]
    pub unsafe fn get_buffer(&self, buffer_id: u16, available_len: usize) -> OwnedBuffer {
        OwnedBuffer {
            pool: self.inner.clone(),
            params: Some((buffer_id, available_len)),
        }
    }

    /// ## Safety
    /// * `len` should be the returned value from the op.
    pub(crate) unsafe fn create_proxy(
        &self,
        slice: OwnedBuffer,
        len: usize,
    ) -> io::Result<BorrowedBuffer<'_>> {
        let Some((buffer_id, available_len)) = slice.leak() else {
            return Err(io::Error::other("no buffer selected"));
        };
        debug_assert_eq!(available_len, len);
        unsafe { self.inner.buf_ring.get_buf(buffer_id, available_len) }
            .map(BorrowedBuffer)
            .ok_or_else(|| io::Error::other(format!("cannot find buffer {buffer_id}")))
    }

    pub(crate) fn reuse_buffer(&self, buffer_id: u16) {
        self.inner.reuse_buffer(buffer_id);
    }
}

#[doc(hidden)]
pub struct OwnedBuffer {
    pool: Rc<BufferPoolInner>,
    params: Option<(u16, usize)>,
}

impl OwnedBuffer {
    pub fn leak(mut self) -> Option<(u16, usize)> {
        self.params.take()
    }
}

impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        if let Some((buffer_id, _)) = self.params {
            self.pool.reuse_buffer(buffer_id);
        }
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
