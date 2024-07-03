use std::{
    fmt::{Debug, Formatter},
    ops::{Deref, DerefMut},
};

use io_uring::cqueue::buffer_select;
use io_uring_buf_ring::IoUringBufRing;

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

    pub unsafe fn get_buffer(&self, flags: u32, available_len: usize) -> Option<BorrowedBuffer> {
        let buffer_id = buffer_select(flags)?;

        self.buf_ring
            .get_buf(buffer_id, available_len)
            .map(BorrowedBuffer)
    }
}

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
