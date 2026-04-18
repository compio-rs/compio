use std::{
    io,
    num::NonZeroU16,
    ptr::{NonNull, null_mut},
    slice,
    sync::atomic::Ordering,
};

use io_uring::types::BufRingEntry;
use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous, munmap};
use synchrony::unsync::atomic::AtomicU16;

use crate::{
    assert_not_impl,
    buffer_pool::{BufPtr, Slot},
};

#[derive(Debug)]
pub(in crate::sys) struct BufControl {
    /// Pointer to the mmap-ed entry memory
    ptr: NonNull<BufRingEntry>,
    /// Number of entries
    len: NonZeroU16,
    /// Total size of the mmap
    size: usize,
}

assert_not_impl!(BufControl, Send);
assert_not_impl!(BufControl, Sync);

impl BufControl {
    const BUF_GROUP: u16 = 1;

    /// # Safety
    ///
    /// Caller must ensure the buffers will:
    ///
    /// - be available throughout the lifetime of this control, and
    /// - not be used (read or write) until a `buffer_id` is returned from
    ///   kernel and the corresponding buffer is taken from `BufferPool`.
    pub unsafe fn new(
        driver: &mut crate::Driver,
        bufs: &[Slot],
        bufs_len: u32,
        flags: u16,
    ) -> io::Result<Self> {
        debug_assert!(bufs.len().is_power_of_two());

        let driver = driver.as_iour_mut().expect("Should be iour");

        let len = NonZeroU16::new(bufs.len() as u16).expect("Empty buffers");
        let size = len.get() as usize * size_of::<BufRingEntry>();

        let prot = ProtFlags::READ | ProtFlags::WRITE;
        let mflags = MapFlags::PRIVATE;
        let ptr = NonNull::new(unsafe { mmap_anonymous(null_mut(), size, prot, mflags) }?)
            .expect("mmap failed")
            .cast::<BufRingEntry>();

        let mut this = Self { ptr, len, size };

        unsafe {
            driver.inner().submitter().register_buf_ring_with_flags(
                ptr.addr().get() as u64,
                len.get(),
                Self::BUF_GROUP,
                flags,
            )
        }?;

        for (id, buf) in bufs.iter().enumerate() {
            let ptr = buf.expect("Cannot initialize with null bufs");
            let id = id as u16;
            unsafe { this.add_buffer(id, ptr, bufs_len, id) };
        }
        // SAFETY: Entries were initialized just now
        unsafe { this.commit(bufs.len() as _) };

        Ok(this)
    }

    pub fn pop(&mut self) -> io::Result<u16> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "pop is not supported on io_uring",
        ))
    }

    /// Get the buffer group id
    pub const fn buffer_group(&self) -> u16 {
        Self::BUF_GROUP
    }

    /// Reset the buffer and make it available to the kernel
    ///
    /// # Safety
    ///
    /// Caller must ensure that the buf is valid until the current ring is
    /// dropped.
    pub unsafe fn reset(&mut self, buffer_id: u16, ptr: BufPtr, len: u32) {
        // SAFETY: Guaranteed by the caller
        unsafe { self.add_buffer(buffer_id, ptr, len, 0) };
        // SAFETY: We just added one buffer
        unsafe { self.commit(1) };
    }

    /// # Safety
    ///
    /// `Self` cannot be used after `release` is being called.
    pub unsafe fn release(&mut self, driver: &mut crate::Driver) -> io::Result<()> {
        let driver = driver.as_iour_mut().expect("Should be iour driver");

        driver
            .inner()
            .submitter()
            .unregister_buf_ring(Self::BUF_GROUP)?;
        unsafe { munmap(self.ptr.cast().as_ptr(), self.size) }?;

        Ok(())
    }

    fn as_slice_mut(&mut self) -> &mut [BufRingEntry] {
        // SAFETY: the pointer is valid and content is zero-init guaranteed by mmap,
        // which is valid memory representation of BufRingEntry
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr().cast(), self.len.get() as _) }
    }

    fn tail(&self) -> &AtomicU16 {
        // SAFETY: Entries are initialized and aligned, and ptr is page-aligned
        // guaranteed by anonymous mmap
        unsafe { &*BufRingEntry::tail(self.ptr.as_ptr()).cast() }
    }

    /// Add a buffer to the ring.
    ///
    /// Calling this alone won't make the buffer visible for kernel to use. Use
    /// [`commit`] to do so.
    ///
    /// # Safety
    ///
    /// Caller must ensure that the buf is valid until the current ring is
    /// dropped.
    ///
    /// [`commit`]: Self::commit
    unsafe fn add_buffer(&mut self, buffer_id: u16, ptr: BufPtr, len: u32, offset: u16) {
        let idx = (self.tail().load(Ordering::Acquire) + offset) % self.len.get();

        let entry = &mut self.as_slice_mut()[idx as usize];

        entry.set_addr(ptr.addr().get() as u64);
        entry.set_len(len as _);
        entry.set_bid(buffer_id);
    }

    /// Commit `count` many buffers for the kernel to use
    ///
    /// # Safety
    ///
    /// Caller must ensure the buffers in range [curr, curr + count) are valid
    /// and not in use.
    unsafe fn commit(&self, count: u16) {
        self.tail().fetch_add(count, Ordering::Release);
    }
}
