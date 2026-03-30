use std::{
    io,
    mem::MaybeUninit,
    num::{NonZeroU16, NonZeroUsize},
    ptr::NonNull,
    slice,
    sync::atomic::Ordering,
};

use io_uring::types::BufRingEntry;
use nix::sys::mman::{MapFlags, ProtFlags, mmap_anonymous, munmap};
use synchrony::unsync::atomic::AtomicU16;

use crate::{assert_not_impl, sys::buffer_pool::Slot};

#[derive(Debug)]
pub struct BufControl {
    /// Pointer to the mmap-ed entry memory
    ptr: NonNull<BufRingEntry>,
    /// Number of entries
    len: NonZeroU16,
    /// Total size of the mmap
    size: NonZeroUsize,
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
    pub unsafe fn new(driver: &mut super::Driver, bufs: &[Slot], flags: u16) -> io::Result<Self> {
        debug_assert!(bufs.len().is_power_of_two());

        let len = NonZeroU16::new(bufs.len() as u16).expect("Empty buffers");
        let size = NonZeroUsize::new(len.get() as usize * size_of::<BufRingEntry>())
            .expect("Shouldn't overflow");

        let prot = ProtFlags::PROT_READ | ProtFlags::PROT_WRITE;
        let mflags = MapFlags::MAP_PRIVATE;
        let ptr = unsafe { mmap_anonymous(None, size, prot, mflags) }?.cast::<BufRingEntry>();

        let mut this = Self { ptr, len, size };

        unsafe {
            driver.inner.submitter().register_buf_ring_with_flags(
                ptr.addr().get() as u64,
                len.get(),
                Self::BUF_GROUP,
                flags,
            )
        }?;

        for (id, buf) in bufs.iter().enumerate() {
            let buf = buf.as_deref().expect("Initialize with empty bufs");
            unsafe { this.add_buffer(id as _, buf) };
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
    pub unsafe fn reset(&mut self, buffer_id: u16, buf: &[MaybeUninit<u8>]) {
        // SAFETY: Guaranteed by the caller
        unsafe { self.add_buffer(buffer_id, buf) };
        // SAFETY: We just added one buffer
        unsafe { self.commit(1) };
    }

    /// # Safety
    ///
    /// `Self` cannot be used after `release` is being called.
    pub unsafe fn release(&mut self, driver: &mut super::Driver) -> io::Result<()> {
        driver
            .inner
            .submitter()
            .unregister_buf_ring(Self::BUF_GROUP)?;
        unsafe { munmap(self.ptr.cast(), self.size.get()) }?;

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
    unsafe fn add_buffer(&mut self, buffer_id: u16, buf: &[MaybeUninit<u8>]) {
        let idx = self.tail().load(Ordering::Acquire) % self.len.get();

        let entry = &mut self.as_slice_mut()[idx as usize];

        entry.set_addr(buf.as_ptr() as _);
        entry.set_len(buf.len() as _);
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
