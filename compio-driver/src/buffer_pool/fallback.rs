use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::VecDeque,
    fmt::{Debug, Formatter},
    io,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use compio_buf::{IntoInner, IoBuf, IoBufMut, SetBufInit, Slice};

struct BufferPoolInner {
    buffers: RefCell<VecDeque<Vec<u8>>>,
}

impl BufferPoolInner {
    pub(crate) fn add_buffer(&self, mut buffer: Vec<u8>) {
        buffer.clear();
        self.buffers.borrow_mut().push_back(buffer)
    }
}

/// Buffer pool
///
/// A buffer pool to allow user no need to specify a specific buffer to do the
/// IO operation
pub struct BufferPool {
    inner: Rc<BufferPoolInner>,
}

impl Debug for BufferPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferPool").finish_non_exhaustive()
    }
}

impl BufferPool {
    pub(crate) fn new(buffer_len: u16, buffer_size: usize) -> Self {
        let buffers = (0..buffer_len.next_power_of_two())
            .map(|_| Vec::with_capacity(buffer_size))
            .collect();

        Self {
            inner: Rc::new(BufferPoolInner {
                buffers: RefCell::new(buffers),
            }),
        }
    }

    pub(crate) fn get_buffer(&self, len: usize) -> io::Result<OwnedBuffer> {
        let buffer = self.inner.buffers.borrow_mut().pop_front().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "buffer ring has no available buffer")
        })?;
        let len = if len == 0 {
            buffer.capacity()
        } else {
            buffer.capacity().min(len)
        };
        Ok(OwnedBuffer::new(buffer.slice(..len), self.inner.clone()))
    }

    pub(crate) fn add_buffer(&self, buffer: Vec<u8>) {
        self.inner.add_buffer(buffer);
    }

    /// Safety: `len` should be valid
    pub(crate) unsafe fn create_proxy(&self, mut slice: OwnedBuffer, len: usize) -> BorrowedBuffer {
        unsafe {
            slice.set_buf_init(len);
        }
        BorrowedBuffer::new(slice.into_inner(), self)
    }
}

pub(crate) struct OwnedBuffer {
    buffer: ManuallyDrop<Slice<Vec<u8>>>,
    pool: Rc<BufferPoolInner>,
}

impl OwnedBuffer {
    fn new(buffer: Slice<Vec<u8>>, pool: Rc<BufferPoolInner>) -> Self {
        Self {
            buffer: ManuallyDrop::new(buffer),
            pool,
        }
    }
}

unsafe impl IoBuf for OwnedBuffer {
    fn as_buf_ptr(&self) -> *const u8 {
        self.buffer.as_buf_ptr()
    }

    fn buf_len(&self) -> usize {
        self.buffer.buf_len()
    }

    fn buf_capacity(&self) -> usize {
        self.buffer.buf_capacity()
    }
}

unsafe impl IoBufMut for OwnedBuffer {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.buffer.as_buf_mut_ptr()
    }
}

impl SetBufInit for OwnedBuffer {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.buffer.set_buf_init(len);
    }
}

impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        self.pool
            .add_buffer(unsafe { ManuallyDrop::take(&mut self.buffer) }.into_inner());
    }
}

impl IntoInner for OwnedBuffer {
    type Inner = Slice<Vec<u8>>;

    fn into_inner(mut self) -> Self::Inner {
        unsafe { ManuallyDrop::take(&mut self.buffer) }
    }
}

/// Buffer borrowed from buffer pool
///
/// When IO operation finish, user will obtain a `BorrowedBuffer` to access the
/// filled data
pub struct BorrowedBuffer<'a> {
    buffer: ManuallyDrop<Slice<Vec<u8>>>,
    pool: &'a BufferPool,
}

impl<'a> BorrowedBuffer<'a> {
    pub(crate) fn new(buffer: Slice<Vec<u8>>, pool: &'a BufferPool) -> Self {
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
