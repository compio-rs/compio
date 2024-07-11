use std::{
    borrow::{Borrow, BorrowMut},
    fmt::{Debug, Formatter},
    ops::{Deref, DerefMut},
};

/// Buffer pool
///
/// A buffer pool to allow user no need to specify a specific buffer to do the
/// IO operation
pub struct BufferPool {
    inner: BufferPollInner,
}

impl Debug for BufferPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish_non_exhaustive()
    }
}

impl BufferPool {
    pub(crate) fn new_io_uring(buffer_pool: super::iour::buffer_pool::BufferPool) -> Self {
        Self {
            inner: BufferPollInner::IoUring(buffer_pool),
        }
    }

    pub(crate) fn as_io_uring(&self) -> &super::iour::buffer_pool::BufferPool {
        match &self.inner {
            BufferPollInner::IoUring(inner) => inner,
            BufferPollInner::Poll(_) => panic!("BufferPool type is not poll type"),
        }
    }

    pub(crate) fn as_poll(&self) -> &crate::fallback_buffer_pool::BufferPool {
        match &self.inner {
            BufferPollInner::Poll(inner) => inner,
            BufferPollInner::IoUring(_) => panic!("BufferPool type is not io-uring type"),
        }
    }

    pub(crate) fn new_poll(buffer_pool: crate::fallback_buffer_pool::BufferPool) -> Self {
        Self {
            inner: BufferPollInner::Poll(buffer_pool),
        }
    }

    pub(crate) fn into_poll(self) -> crate::fallback_buffer_pool::BufferPool {
        match self.inner {
            BufferPollInner::IoUring(_) => {
                panic!("BufferPool type is not io-uring type")
            }
            BufferPollInner::Poll(inner) => inner,
        }
    }

    pub(crate) fn into_io_uring(self) -> super::iour::buffer_pool::BufferPool {
        match self.inner {
            BufferPollInner::IoUring(inner) => inner,
            BufferPollInner::Poll(_) => panic!("BufferPool type is not poll type"),
        }
    }
}

enum BufferPollInner {
    IoUring(super::iour::buffer_pool::BufferPool),
    Poll(crate::fallback_buffer_pool::BufferPool),
}

/// Buffer borrowed from buffer pool
///
/// When IO operation finish, user will obtain a `BorrowedBuffer` to access the
/// filled data
pub struct BorrowedBuffer<'a> {
    inner: BorrowedBufferInner<'a>,
}

impl<'a> BorrowedBuffer<'a> {
    pub(crate) fn new_io_uring(buffer: super::iour::buffer_pool::BorrowedBuffer<'a>) -> Self {
        Self {
            inner: BorrowedBufferInner::IoUring(buffer),
        }
    }

    pub(crate) fn new_poll(buffer: crate::fallback_buffer_pool::BorrowedBuffer<'a>) -> Self {
        Self {
            inner: BorrowedBufferInner::Poll(buffer),
        }
    }
}

enum BorrowedBufferInner<'a> {
    IoUring(super::iour::buffer_pool::BorrowedBuffer<'a>),
    Poll(crate::fallback_buffer_pool::BorrowedBuffer<'a>),
}

impl Debug for BorrowedBuffer<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorrowedBuffer").finish_non_exhaustive()
    }
}

impl Deref for BorrowedBuffer<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            BorrowedBufferInner::IoUring(inner) => inner.deref(),
            BorrowedBufferInner::Poll(inner) => inner.deref(),
        }
    }
}

impl DerefMut for BorrowedBuffer<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.inner {
            BorrowedBufferInner::IoUring(inner) => inner.deref_mut(),
            BorrowedBufferInner::Poll(inner) => inner.deref_mut(),
        }
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
