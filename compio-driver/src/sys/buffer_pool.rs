use std::{
    cell::UnsafeCell,
    fmt::Debug,
    io,
    mem::{self, MaybeUninit},
    ops::{Deref, DerefMut},
    ptr::{self, NonNull},
    rc::{Rc, Weak},
    slice,
};

use compio_buf::{IoBuf, IoBufMut, SetLen};

/// A buffer pointer without length part.
pub(crate) type BufPtr = NonNull<MaybeUninit<u8>>;
/// A buffer slot. It's always 1-pointer sized thanks to niche optimization.
pub(crate) type Slot = Option<BufPtr>;

const _: () = assert!(size_of::<Slot>() == size_of::<usize>());

cfg_if::cfg_if! {
    if #[cfg(io_uring)] {
        use super::imp::BufControl;

        unsafe fn create_buf_control(
            driver: &mut super::Driver,
            bufs: &[Slot],
            buf_len: u32,
            flags: u16
        ) -> io::Result<BufControl> {
            unsafe { BufControl::new(driver, bufs, buf_len, flags) }
        }
    } else {
        use fallback::BufControl;

        unsafe fn create_buf_control(
            _: &mut super::Driver,
            bufs: &[Slot],
            _: u32,
            _: u16
        ) -> io::Result<BufControl> {
            Ok(BufControl::new(bufs))
        }
    }
}

/// A buffer pool.
///
/// This type by itself does nothing, and should only be used by `*Managed` ops.
#[derive(Clone)]
pub struct BufferPool {
    shared: Weak<Shared>,
}

#[repr(transparent)]
#[derive(Debug)]
pub(crate) struct BufferPoolRoot {
    shared: Rc<Shared>,
}

/// A unique reference to a buffer within the buffer pool.
///
/// Dropping this type will reset the buffer back to the pool instead of
/// releasing buffer's memory.
#[derive(Debug)]
pub struct BufferRef {
    /// Initialized length of the buffer, set with [`SetLen`]
    len: u32,
    /// User-set capacity, default to `full_cap`
    cap: u32,
    /// Full capacity of the buffer, used to release memory if driver (buffer
    /// pool) is dropped
    full_cap: u32,
    /// Weak handle of the buffer pool
    shared: Weak<Shared>,
    /// Pointer of the buffer
    ptr: BufPtr,
    /// Buffer id (index within the Vec)
    buffer_id: u16,
}

#[repr(transparent)]
struct Shared {
    inner: UnsafeCell<Inner>,
}

struct Inner {
    /// Control block corresponds to each driver
    ctrl: BufControl,

    /// Size of each buffer
    size: u32,

    /// Buffer pointers
    bufs: Vec<Slot>,
}

impl BufferPoolRoot {
    pub(crate) fn new(
        driver: &mut crate::Driver,
        num_of_bufs: u16,
        buffer_size: usize,
        flags: u16,
    ) -> io::Result<Self> {
        let size: u32 = buffer_size.try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Buffer size too large. Should be able to fit into u32.",
            )
        })?;
        let buffers = (0..num_of_bufs.next_power_of_two())
            .map(|_| {
                let ptr = Box::into_raw(Box::<[u8]>::new_uninit_slice(buffer_size)).cast();
                // SAFETY: Creating NonNull from Box
                Some(unsafe { NonNull::new_unchecked(ptr) })
            })
            .collect::<Vec<_>>();
        let control = unsafe { create_buf_control(driver, &buffers, size, flags) }?;

        Ok(Self {
            shared: Shared {
                inner: Inner {
                    ctrl: control,
                    size,
                    bufs: buffers,
                }
                .into(),
            }
            .into(),
        })
    }

    /// Release the buffer pool and deallocate all buffers.
    ///
    /// If the buffer pool root is dropped without calling this function,
    /// everything will be leaked and there will be no chance to recover them
    /// back, except those have been taken by [`BufferRef`], which will be
    /// released when they're dropped.
    ///
    /// If the control block failed to release, this function will return an io
    /// Error without deallocating buffers, and it's possible to retry.
    ///
    /// # Safety
    ///
    /// [`BufferPoolRoot`] must not be used after `release` is called and
    /// returned successfully. Only thing that's safe to do afterwards is to
    /// drop it.
    pub(crate) unsafe fn release(&mut self, driver: &mut crate::Driver) -> io::Result<()> {
        let (bufs, size) = unsafe {
            self.shared.with(|inner| {
                inner.ctrl.release(driver)?;
                io::Result::Ok((mem::take(&mut inner.bufs), inner.size))
            })
        }?;

        // Control is successfully released, now deallocate buffers
        for ptr in bufs.into_iter().flatten() {
            let ptr = ptr::slice_from_raw_parts_mut(ptr.as_ptr(), size as usize);
            _ = unsafe { Box::from_raw(ptr) };
        }

        Ok(())
    }

    pub(crate) fn get_pool(&self) -> BufferPool {
        BufferPool {
            shared: Rc::downgrade(&self.shared),
        }
    }

    pub(crate) fn is_unique(&self) -> bool {
        Rc::strong_count(&self.shared) == 1
    }
}

impl Debug for BufferPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(shared) = self.shared.upgrade() {
            f.debug_struct("BufferPool")
                .field("shared", &shared)
                .finish()
        } else {
            f.debug_struct("BufferPool")
                .field("shared", &"<dropped>")
                .finish()
        }
    }
}

impl Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct Buf {
            ptr: BufPtr,
        }

        impl Debug for Buf {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Buf<{:p}>", self.ptr)
            }
        }

        struct BuffersDebug<'a> {
            buffers: &'a [Slot],
        }

        impl Debug for BuffersDebug<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_list()
                    .entries(self.buffers.iter().map(|buf| buf.map(|ptr| Buf { ptr })))
                    .finish()
            }
        }

        unsafe {
            self.with(|inner| {
                let buffers = BuffersDebug {
                    buffers: &inner.bufs,
                };
                f.debug_struct("Shared")
                    .field("control", &inner.ctrl)
                    .field("size", &inner.size)
                    .field("buffers", &buffers)
                    .finish()
            })
        }
    }
}

impl BufferPool {
    /// Pop an available buffer from the pool with given capacity.
    ///
    /// This operation is not supported on io-uring driver and will always
    /// return [`Unsupported`].
    ///
    /// [`Unsupported`]: io::ErrorKind::Unsupported
    pub fn pop(&self) -> io::Result<BufferRef> {
        let buffer_id = unsafe { self.with(|inner| inner.ctrl.pop()) }??;

        Ok(self.take(buffer_id)?.expect("Buffer should be available"))
    }

    /// Take the indicated buffer from the pool.
    ///
    /// Returns `None` if the buffer is not reset back yet or does not exist.
    pub fn take(&self, buffer_id: u16) -> io::Result<Option<BufferRef>> {
        let shared = self.shared()?;
        let Some(ptr) = shared.take(buffer_id) else {
            return Ok(None);
        };
        let cap = shared.len();

        Ok(Some(BufferRef {
            len: 0,
            cap,
            full_cap: cap,
            shared: Rc::downgrade(&shared),
            ptr,
            buffer_id,
        }))
    }

    /// Reset the `buffer_id` so that it's available for kernel to use, return
    /// whether a buffer has been reset.
    ///
    /// This is the same as `take(buffer_id)` and immediately drop it.
    pub fn reset(&self, buffer_id: u16) -> io::Result<bool> {
        let shared = self.shared()?;
        let Some(buf) = shared.take(buffer_id) else {
            return Ok(false);
        };
        shared.reset(buffer_id, buf);
        Ok(true)
    }

    fn shared(&self) -> io::Result<Rc<Shared>> {
        self.shared
            .upgrade()
            .ok_or_else(|| io::Error::other("The driver has been dropped"))
    }

    /// # Safety
    ///
    /// `f` must not access `self` reentrantly
    unsafe fn with<F, R>(&self, f: F) -> io::Result<R>
    where
        F: FnOnce(&mut Inner) -> R,
    {
        Ok(unsafe { self.shared()?.with(f) })
    }

    /// Get the group id of this buffer pool.
    #[cfg(io_uring)]
    pub(crate) fn buffer_group(&self) -> io::Result<u16> {
        unsafe { self.with(|i| i.ctrl.buffer_group()) }
    }

    /// Test if the buffer pool is an io_uring one.
    #[cfg(fusion)]
    pub fn is_io_uring(&self) -> io::Result<bool> {
        unsafe { self.with(|inner| inner.ctrl.is_io_uring()) }
    }
}

impl Shared {
    /// # Safety
    ///
    /// `f` must not access [`Self::inner`] reentrantly
    #[inline(always)]
    unsafe fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Inner) -> R,
    {
        f(unsafe { &mut *self.inner.get() })
    }

    fn take(&self, buffer_id: u16) -> Option<BufPtr> {
        unsafe { self.with(|inner| inner.bufs[buffer_id as usize].take()) }
    }

    fn reset(&self, buffer_id: u16, ptr: BufPtr) {
        unsafe {
            self.with(|inner| {
                inner.ctrl.reset(buffer_id, ptr, inner.size);
                inner.bufs[buffer_id as usize] = Some(ptr);
            })
        }
    }

    fn len(&self) -> u32 {
        unsafe { self.with(|inner| inner.size) }
    }
}

impl BufferRef {
    /// Set the capacity of this buffer.
    ///
    /// This does nothing if `cap` is greater than underlying buffer's
    /// length.
    pub fn with_capacity(mut self, cap: usize) -> Self {
        self.set_capacity(cap);
        self
    }

    /// Set the capacity of this buffer.
    ///
    /// This does nothing if `cap` is greater than underlying buffer's
    /// length or equals 0.
    pub fn set_capacity(&mut self, cap: usize) {
        if cap == 0 {
            return;
        }
        self.cap = (cap as u32).min(self.full_cap);
        self.len = self.len.min(self.cap);
    }
}

impl Deref for BufferRef {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // SAFETY: `SetLen` guarantees the range is initialized
        unsafe { slice::from_raw_parts(self.ptr.as_ptr().cast(), self.len as usize) }
    }
}

impl DerefMut for BufferRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `SetLen` guarantees the range is initialized
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr() as _, self.len as usize) }
    }
}

impl IoBuf for BufferRef {
    fn as_init(&self) -> &[u8] {
        self
    }
}

impl SetLen for BufferRef {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= u32::MAX as usize);
        self.len = (len as u32).min(self.cap);
    }
}

impl IoBufMut for BufferRef {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        // SAFETY: Cap is initialized as the buffer length, and setting it is
        // is capped at full_cap, so it will never exceed buffer length. Pointer is
        // not deallocated.
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.cap as usize) }
    }
}

impl Drop for BufferRef {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            // If the buffer pool is alive, set the pointer back
            shared.reset(self.buffer_id, self.ptr);
        } else {
            // Otherwise, drop the buffer
            let ptr = ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), self.full_cap as usize);
            _ = unsafe { Box::from_raw(ptr) };
        }
    }
}
#[cfg(any(fusion, not(io_uring)))]
pub(crate) mod fallback {
    use std::{collections::VecDeque, io};

    use super::*;

    #[derive(Debug)]
    pub struct BufControl {
        queue: VecDeque<u16>,
    }

    impl BufControl {
        pub fn new(bufs: &[Slot]) -> Self {
            assert!(bufs.len() < u16::MAX as _);
            Self {
                queue: bufs.iter().enumerate().map(|(id, _)| id as u16).collect(),
            }
        }

        #[allow(dead_code)]
        pub unsafe fn release(&mut self, _: &mut crate::Driver) -> io::Result<()> {
            Ok(())
        }

        pub fn pop(&mut self) -> io::Result<u16> {
            self.queue
                .pop_front()
                .ok_or_else(|| io::Error::other("buffer ring has no available buffer"))
        }

        pub unsafe fn reset(&mut self, buffer_id: u16, _: BufPtr, _: u32) {
            self.queue.push_back(buffer_id);
        }
    }
}
