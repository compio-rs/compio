//! Io-uring ring mapped buffers implement

use std::{
    borrow::Borrow,
    cell::Cell,
    fmt, io,
    ops::Deref,
    ptr,
    rc::Rc,
    sync::atomic::{self, AtomicU16},
};

use io_uring::{cqueue, squeue, types::BufRingEntry, IoUring};

use crate::Proactor;

/// An anonymous region of memory mapped using `mmap(2)`, not backed by a file
/// but that is guaranteed to be page-aligned and zero-filled.
pub struct AnonymousMmap {
    addr: ptr::NonNull<libc::c_void>,
    len: usize,
}

impl AnonymousMmap {
    /// Allocate `len` bytes that are page aligned and zero-filled.
    pub fn new(len: usize) -> io::Result<AnonymousMmap> {
        unsafe {
            match libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_ANONYMOUS | libc::MAP_SHARED | libc::MAP_POPULATE,
                -1,
                0,
            ) {
                libc::MAP_FAILED => Err(io::Error::last_os_error()),
                addr => {
                    // here, `mmap` will never return null
                    let addr = ptr::NonNull::new_unchecked(addr);
                    Ok(AnonymousMmap { addr, len })
                }
            }
        }
    }

    /// Do not make the stored memory accessible by child processes after a
    /// `fork`.
    pub fn dont_fork(&self) -> io::Result<()> {
        match unsafe { libc::madvise(self.addr.as_ptr(), self.len, libc::MADV_DONTFORK) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Get a pointer to the memory.
    #[inline]
    pub fn as_ptr(&self) -> *const libc::c_void {
        self.addr.as_ptr()
    }

    /// Get a mut pointer to the memory.
    #[inline]
    pub fn as_ptr_mut(&self) -> *mut libc::c_void {
        self.addr.as_ptr()
    }

    /// Get a pointer to the data at the given offset.
    #[inline]
    #[allow(dead_code)]
    pub unsafe fn offset(&self, offset: u32) -> *const libc::c_void {
        self.as_ptr().add(offset as usize)
    }

    /// Get a mut pointer to the data at the given offset.
    #[inline]
    #[allow(dead_code)]
    pub unsafe fn offset_mut(&self, offset: u32) -> *mut libc::c_void {
        self.as_ptr_mut().add(offset as usize)
    }
}

impl Drop for AnonymousMmap {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.addr.as_ptr(), self.len);
        }
    }
}

struct InnerBufRing {
    // All these fields are constant once the struct is instantiated except the one of type
    // Cell<u16>.
    bgid: u16,

    ring_entries_mask: u16,
    // Invariant one less than ring_entries which is > 0, power of 2,
    // max 2^15 (32768).
    buf_cnt: u16,
    // Invariants: > 0, <= ring_entries.
    buf_len: usize, // Invariant: > 0.

    // `ring_start` holds the memory allocated for the buf_ring, the ring of entries describing
    // the buffers being made available to the uring interface for this buf group id.
    ring_start: AnonymousMmap,

    buf_list: Vec<Vec<u8>>,

    // `local_tail` is the copy of the tail index that we update when a buffer is dropped and
    // therefore its buffer id is released and added back to the ring. It also serves for adding
    // buffers to the ring during init but that's not as interesting.
    local_tail: Cell<u16>,

    // `shared_tail` points to the u16 memory inside the rings that the uring interface uses as the
    // tail field. It is where the application writes new tail values and the kernel reads the tail
    // value from time to time. The address could be computed from ring_start when needed. This
    // might be here for no good reason any more.
    shared_tail: *const AtomicU16,
}

impl InnerBufRing {
    fn new(bgid: u16, ring_entries: u16, buf_cnt: u16, buf_len: usize) -> io::Result<InnerBufRing> {
        // Check that none of the important args are zero and the ring_entries is at
        // least large enough to hold all the buffers and that ring_entries is a
        // power of 2.
        if (buf_cnt == 0)
            || (buf_cnt > ring_entries)
            || (buf_len == 0)
            || ((ring_entries & (ring_entries - 1)) != 0)
        {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }

        // entry_size is 16 bytes.
        let entry_size = std::mem::size_of::<BufRingEntry>();
        assert_eq!(entry_size, 16);
        let ring_size = entry_size * (ring_entries as usize);

        // The memory is required to be page aligned and zero-filled by the uring
        // buf_ring interface. Anonymous mmap promises both of those things.
        // https://man7.org/linux/man-pages/man2/mmap.2.html
        let ring_start = AnonymousMmap::new(ring_size).unwrap();
        ring_start.dont_fork()?;

        // Probably some functional way to do this.
        let buf_list: Vec<Vec<u8>> = {
            let mut bp = Vec::with_capacity(buf_cnt as _);
            for _ in 0..buf_cnt {
                bp.push(vec![0; buf_len]);
            }
            bp
        };

        let shared_tail = unsafe { BufRingEntry::tail(ring_start.as_ptr() as *const BufRingEntry) }
            as *const AtomicU16;

        let ring_entries_mask = ring_entries - 1;
        assert_eq!((ring_entries & ring_entries_mask), 0);

        let buf_ring = InnerBufRing {
            bgid,
            ring_entries_mask,
            buf_cnt,
            buf_len,
            ring_start,
            buf_list,
            local_tail: Cell::new(0),
            shared_tail,
        };

        Ok(buf_ring)
    }

    /// Register the buffer ring with the uring interface.
    /// Normally this is done automatically when building a BufRing.
    ///
    /// Warning: requires the CURRENT driver is already in place or will panic.
    fn register<S, C>(&self, ring: &mut IoUring<S, C>) -> io::Result<()>
    where
        S: squeue::EntryMarker,
        C: cqueue::EntryMarker,
    {
        let bgid = self.bgid;

        // Safety: The ring, represented by the ring_start and the ring_entries remains
        // valid until it is unregistered. The backing store is an AnonymousMmap
        // which remains valid until it is dropped which in this case, is when
        // Self is dropped.
        let res = unsafe {
            ring.submitter().register_buf_ring(
                self.ring_start.as_ptr() as _,
                self.ring_entries(),
                bgid,
            )
        };

        if let Err(e) = res {
            return match e.raw_os_error() {
                Some(libc::EINVAL) => {
                    // using buf_ring requires kernel 5.19 or greater.
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "buf_ring.register returned {}, most likely indicating this kernel is \
                             not 5.19+",
                            e
                        ),
                    ))
                }
                Some(libc::EEXIST) => {
                    // Registering a duplicate bgid is not allowed. There is an `unregister`
                    // operations that can remove the first, but care must be taken that there
                    // are no outstanding operations that will still return a buffer from that
                    // one.
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "buf_ring.register returned `{}`, indicating the attempted buffer \
                             group id {} was already registered",
                            e, bgid
                        ),
                    ))
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("buf_ring.register returned `{}` for group id {}", e, bgid),
                )),
            };
        };

        // Add the buffers after the registration. Really seems it could be done earlier
        // too.

        for bid in 0..self.buf_cnt {
            self.buf_ring_push(bid);
        }
        self.buf_ring_sync();

        res
    }

    // Unregister the buffer ring from the io_uring.
    // Normally this is done automatically when the BufRing goes out of scope.
    fn unregister<S, C>(&self, ring: &mut IoUring<S, C>) -> io::Result<()>
    where
        S: squeue::EntryMarker,
        C: cqueue::EntryMarker,
    {
        let bgid = self.bgid;

        ring.submitter().unregister_buf_ring(bgid)
    }

    // Returns the buffer group id.
    fn bgid(&self) -> u16 {
        self.bgid
    }

    // Returns the buffer the uring interface picked from the buf_ring for the
    // completion result represented by the res and flags.
    fn get_buf(
        &self,
        buf_ring: RawRingMappedBuffers,
        res: u32,
        flags: u32,
    ) -> io::Result<RingMappedBuffer> {
        // This fn does the odd thing of having self as the BufRing and taking an
        // argument that is the same BufRing but wrapped in Rc<_> so the wrapped
        // buf_ring can be passed to the outgoing GBuf.

        let bid = cqueue::buffer_select(flags).unwrap();

        let len = res as usize;

        assert!(len <= self.buf_len);

        Ok(RingMappedBuffer::new(buf_ring, bid, len))
    }

    // Safety: dropping a duplicate bid is likely to cause undefined behavior
    // as the kernel could use the same buffer for different data concurrently.
    unsafe fn dropping_bid(&self, bid: u16) {
        self.buf_ring_push(bid);
        self.buf_ring_sync();
    }

    fn buf_capacity(&self) -> usize {
        self.buf_len as _
    }

    fn stable_ptr(&self, bid: u16) -> *const u8 {
        self.buf_list[bid as usize].as_ptr()
    }

    fn ring_entries(&self) -> u16 {
        self.ring_entries_mask + 1
    }

    fn mask(&self) -> u16 {
        self.ring_entries_mask
    }

    // Push the `bid` buffer to the buf_ring tail.
    // This test version does not safeguard against a duplicate
    // `bid` being pushed.
    fn buf_ring_push(&self, bid: u16) {
        assert!(bid < self.buf_cnt);

        // N.B. The uring buf_ring indexing mechanism calls for the tail values to
        // exceed the actual number of ring entries. This allows the uring
        // interface to distinguish between empty and full buf_rings. As a
        // result, the ring mask is only applied to the index used for computing
        // the ring entry, not to the tail value itself.

        let old_tail = self.local_tail.get();
        self.local_tail.set(old_tail + 1);
        let ring_idx = old_tail & self.mask();

        let entries = self.ring_start.as_ptr_mut() as *mut BufRingEntry;
        let re = unsafe { &mut *entries.add(ring_idx as usize) };

        re.set_addr(self.stable_ptr(bid) as _);
        re.set_len(self.buf_len as _);
        re.set_bid(bid);

        // Also note, we have not updated the tail as far as the kernel is
        // concerned. That is done with buf_ring_sync.
    }

    // Make 'local_tail' visible to the kernel. Called after buf_ring_push() has
    // been called to fill in new buffers.
    fn buf_ring_sync(&self) {
        unsafe {
            (*self.shared_tail).store(self.local_tail.get(), atomic::Ordering::Release);
        }
    }
}

#[derive(Clone)]
pub struct RawRingMappedBuffers {
    // The BufRing is reference counted because each buffer handed out has a reference back to its
    // buffer group, or in this case, to its buffer ring.
    rc: Rc<InnerBufRing>,
}

impl RawRingMappedBuffers {
    fn new(buf_ring: InnerBufRing) -> Self {
        RawRingMappedBuffers {
            rc: Rc::new(buf_ring),
        }
    }

    pub fn buf_len(&self) -> usize {
        self.rc.buf_len
    }

    pub fn bgid(&self) -> u16 {
        self.rc.bgid
    }

    #[doc(hidden)]
    /// Safety: user should make sure the `len` is correct
    pub unsafe fn get_buf(&self, len: u32, flags: u32) -> io::Result<RingMappedBuffer> {
        self.rc.get_buf(self.clone(), len, flags)
    }

    #[doc(hidden)]
    pub fn unregister(&mut self, proactor: &mut Proactor) -> io::Result<()> {
        if Rc::strong_count(&self.rc) == 1 {
            self.rc.unregister(proactor.driver().ring())
        } else {
            Ok(())
        }
    }
}

/// The Builder for a ring_mapped_buffers.
#[derive(Copy, Clone)]
pub struct Builder {
    bgid: u16,
    ring_entries: u16,
    buf_cnt: u16,
    buf_len: usize,
}

impl Builder {
    /// Create a new Builder with the given buffer group ID and defaults.
    ///
    /// The buffer group ID, `bgid`, is the id the kernel uses to identify the
    /// buffer group to use for a given read operation that has been placed
    /// into an sqe.
    ///
    /// The caller is responsible for picking a bgid that does not conflict with
    /// other buffer groups that have been registered with the same uring
    /// interface.
    pub fn new(bgid: u16) -> Builder {
        Builder {
            bgid,
            ring_entries: 128,
            buf_cnt: 0, // 0 indicates buf_cnt is taken from ring_entries
            buf_len: 4096,
        }
    }

    /// The number of ring entries to create for the buffer ring.
    ///
    /// The number will be made a power of 2, and will be the maximum of the
    /// ring_entries setting and the buf_cnt setting. The interface will
    /// enforce a maximum of 2^15 (32768).
    pub fn ring_entries(mut self, ring_entries: u16) -> Builder {
        self.ring_entries = ring_entries;
        self
    }

    /// The number of buffers to allocate. If left zero, the ring_entries value
    /// will be used.
    pub fn buf_cnt(mut self, buf_cnt: u16) -> Builder {
        self.buf_cnt = buf_cnt;
        self
    }

    /// The length to be pre-allocated for each buffer.
    pub fn buf_len(mut self, buf_len: usize) -> Builder {
        self.buf_len = buf_len;
        self
    }

    /// Return a RingMappedBuffers.
    #[doc(hidden)]
    pub fn build(&self, proactor: &mut Proactor) -> io::Result<RawRingMappedBuffers> {
        let mut b: Builder = *self;

        // Two cases where both buf_cnt and ring_entries are set to the max of the two.
        if b.buf_cnt == 0 || b.ring_entries < b.buf_cnt {
            let max = std::cmp::max(b.ring_entries, b.buf_cnt);
            b.buf_cnt = max;
            b.ring_entries = max;
        }

        // Don't allow the next_power_of_two calculation to be done if already larger
        // than 2^15 because 2^16 reads back as 0 in a u16. The interface
        // doesn't allow for ring_entries larger than 2^15 anyway, so this is a
        // good place to catch it. Here we return a unique error that is more
        // descriptive than the InvalidArg that would come from the interface.
        if b.ring_entries > (1 << 15) {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "ring_entries exceeded 32768",
            ));
        }

        // Requirement of the interface is the ring entries is a power of two, making
        // its and our wrap calculation trivial.
        b.ring_entries = b.ring_entries.next_power_of_two();

        let inner = InnerBufRing::new(b.bgid, b.ring_entries, b.buf_cnt, b.buf_len)?;
        inner.register(proactor.driver().ring())?;

        Ok(RawRingMappedBuffers::new(inner))
    }
}

/// This tracks a buffer that has been filled in by the kernel, having gotten
/// the memory from a buffer ring, and returned to userland via a cqe entry.
pub struct RingMappedBuffer {
    buf_group: RawRingMappedBuffers,
    len: usize,
    bid: u16,
}

impl fmt::Debug for RingMappedBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RingMappedBuffer")
            .field("bgid", &self.buf_group.rc.bgid())
            .field("bid", &self.bid)
            .field("len", &self.len)
            .field("cap", &self.buf_group.rc.buf_capacity())
            .finish()
    }
}

impl RingMappedBuffer {
    fn new(buf_group: RawRingMappedBuffers, bid: u16, len: usize) -> Self {
        assert!(len <= buf_group.rc.buf_len);

        Self {
            buf_group,
            len,
            bid,
        }
    }

    /// Return the capacity of this buffer.
    pub fn cap(&self) -> usize {
        self.buf_group.rc.buf_capacity()
    }

    /// Return a byte slice reference.
    fn as_slice(&self) -> &[u8] {
        let p = self.buf_group.rc.stable_ptr(self.bid);
        unsafe { std::slice::from_raw_parts(p, self.len) }
    }
}

impl Deref for RingMappedBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl Drop for RingMappedBuffer {
    fn drop(&mut self) {
        // Add the buffer back to the bufgroup, for the kernel to reuse.
        unsafe { self.buf_group.rc.dropping_bid(self.bid) };
    }
}

impl Borrow<[u8]> for RingMappedBuffer {
    fn borrow(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for RingMappedBuffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}
