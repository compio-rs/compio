use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::VecDeque,
    fmt::{Debug, Formatter},
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};

use compio_buf::{IntoInner, Slice};

pub struct BufferPool {
    buffers: RefCell<VecDeque<Vec<u8>>>,
}

impl Debug for BufferPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish_non_exhaustive()
    }
}

impl BufferPool {
    pub(crate) fn new(buffer_len: u16, buffer_size: usize) -> Self {
        let buffers = (0..buffer_len)
            .map(|_| Vec::with_capacity(buffer_size))
            .collect();

        Self {
            buffers: RefCell::new(buffers),
        }
    }

    pub fn get_buffer(&self) -> Option<Vec<u8>> {
        self.buffers.borrow_mut().pop_front()
    }

    pub fn add_buffer(&self, buffer: Vec<u8>) {
        self.buffers.borrow_mut().push_back(buffer)
    }
}

pub struct BorrowedBuffer<'a> {
    buffer: ManuallyDrop<Slice<Vec<u8>>>,
    pool: &'a BufferPool,
}

impl<'a> BorrowedBuffer<'a> {
    pub fn new(buffer: Slice<Vec<u8>>, pool: &'a BufferPool) -> Self {
        Self {
            buffer: ManuallyDrop::new(buffer),
            pool,
        }
    }
}

impl Debug for BorrowedBuffer<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorrowedBuffer").finish_non_exhaustive()
    }
}

impl Drop for BorrowedBuffer<'_> {
    fn drop(&mut self) {
        let buffer = unsafe {
            // Safety: we won't take self.buffer again
            ManuallyDrop::take(&mut self.buffer)
        };
        self.pool.add_buffer(buffer.into_inner());
    }
}

impl Deref for BorrowedBuffer<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.buffer.deref()
    }
}

impl DerefMut for BorrowedBuffer<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer.deref_mut()
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
