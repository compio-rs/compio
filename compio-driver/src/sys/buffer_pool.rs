use std::{
    cell::UnsafeCell,
    fmt::Debug,
    io,
    mem::{ManuallyDrop, MaybeUninit},
    ops::{Deref, DerefMut},
    rc::{Rc, Weak},
    slice,
};

use compio_buf::{IoBuf, IoBufMut, SetLen};

/// A buffer slot. It's always 2-pointer sized thanks to niche optimization.
pub(crate) type Slot = Option<Box<[MaybeUninit<u8>]>>;

const _: () = assert!(size_of::<Slot>() == 2 * size_of::<usize>());

cfg_if::cfg_if! {
    if #[cfg(io_uring)] {
        use super::imp::BufControl;

        unsafe fn create_buf_control(
            driver: &mut super::Driver,
            bufs: &[Option<Box<[MaybeUninit<u8>]>>],
            flags: u16
        ) -> io::Result<BufControl> {
            unsafe { BufControl::new(driver, bufs, flags) }
        }
    } else {
        use fallback::BufControl;

        unsafe fn create_buf_control(
            _: &mut super::Driver,
            bufs: &[Option<Box<[MaybeUninit<u8>]>>],
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
/// releasing buffer's memeory.
pub struct BufferRef {
    len: usize,
    cap: usize,
    shared: Weak<Shared>,
    buffer: ManuallyDrop<Box<[MaybeUninit<u8>]>>,
    buffer_id: u16,
}

#[repr(transparent)]
struct Shared {
    inner: UnsafeCell<Inner>,
}

struct Inner {
    control: BufControl,
    buffers: Vec<Slot>,
}

impl BufferPoolRoot {
    pub(crate) fn new(
        driver: &mut crate::Driver,
        num_of_bufs: u16,
        buffer_size: usize,
        flags: u16,
    ) -> io::Result<Self> {
        let buffers = (0..num_of_bufs.next_power_of_two())
            .map(|_| Some(Box::new_uninit_slice(buffer_size)))
            .collect::<Vec<_>>();
        let control = unsafe { create_buf_control(driver, &buffers, flags) }?;

        Ok(Self {
            shared: Shared {
                inner: Inner { control, buffers }.into(),
            }
            .into(),
        })
    }

    /// # Safety
    ///
    /// [`BufferPoolRoot`] must not be used after `release` is called. Only
    /// thing that's safe to do afterwards is to drop it.
    pub(crate) unsafe fn release(&mut self, driver: &mut crate::Driver) -> io::Result<()> {
        unsafe { self.shared.with(|inner| inner.control.release(driver)) }
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
            len: usize,
        }

        impl Debug for Buf {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Buf<{}>", self.len)
            }
        }

        struct BuffersDebug<'a> {
            buffers: &'a [Option<Box<[MaybeUninit<u8>]>>],
        }

        impl Debug for BuffersDebug<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_list()
                    .entries(
                        self.buffers
                            .iter()
                            .map(|buf| buf.as_ref().map(|b| Buf { len: b.len() })),
                    )
                    .finish()
            }
        }

        unsafe {
            self.with(|inner| {
                let buffers = BuffersDebug {
                    buffers: &inner.buffers,
                };
                f.debug_struct("Shared")
                    .field("control", &inner.control)
                    .field("size", &inner.buffers.len())
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
        let buffer_id = unsafe { self.with(|inner| inner.control.pop()) }??;

        Ok(self.take(buffer_id)?.expect("Buffer should be available"))
    }

    /// Take the indicated buffer from the pool.
    ///
    /// Returns `None` if the buffer is not reset back yet or does not exist.
    pub fn take(&self, buffer_id: u16) -> io::Result<Option<BufferRef>> {
        let shared = self.shared()?;
        let Some(buffer) = shared.take(buffer_id) else {
            return Ok(None);
        };

        Ok(Some(BufferRef {
            len: 0,
            cap: buffer.len(),
            shared: Rc::downgrade(&shared),
            buffer: ManuallyDrop::new(buffer),
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
        unsafe { self.with(|i| i.control.buffer_group()) }
    }

    /// Test if the buffer pool is an io_uring one.
    #[cfg(fusion)]
    pub fn is_io_uring(&self) -> io::Result<bool> {
        unsafe { self.with(|inner| inner.control.is_io_uring()) }
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

    fn take(&self, buffer_id: u16) -> Option<Box<[MaybeUninit<u8>]>> {
        unsafe { self.with(|inner| inner.buffers[buffer_id as usize].take()) }
    }

    fn reset(&self, buffer_id: u16, buffer: Box<[MaybeUninit<u8>]>) {
        unsafe {
            self.with(|inner| {
                inner.control.reset(buffer_id, &buffer);
                inner.buffers[buffer_id as usize] = Some(buffer);
            })
        }
    }
}

impl BufferRef {
    /// Set the capacity of this buffer.
    ///
    /// This will does nothing if `cap` is greater than underlying buffer's
    /// length.
    ///
    /// # Panic
    ///
    /// Panic if `cap <= self.len`
    pub fn with_capacity(mut self, cap: usize) -> Self {
        self.set_capacity(cap);
        self
    }

    /// Set the capacity of this buffer.
    ///
    /// This will does nothing if `cap` is greater than underlying buffer's
    /// length or equals 0.
    pub fn set_capacity(&mut self, cap: usize) {
        if cap == 0 {
            return;
        }
        self.cap = cap.min(self.buffer.len());
        self.len = self.len.min(self.cap);
    }
}

impl Deref for BufferRef {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // SAFETY: `SetLen` guarantees the range is initialized
        unsafe { slice::from_raw_parts(self.buffer.as_ptr().cast(), self.len) }
    }
}

impl DerefMut for BufferRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `SetLen` guarantees the range is initialized
        unsafe { slice::from_raw_parts_mut(self.buffer.as_ptr() as _, self.len) }
    }
}

impl IoBuf for BufferRef {
    fn as_init(&self) -> &[u8] {
        self
    }
}

impl SetLen for BufferRef {
    unsafe fn set_len(&mut self, len: usize) {
        self.len = len.min(self.cap);
    }
}

impl IoBufMut for BufferRef {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        &mut self.buffer[..self.cap]
    }
}

impl Drop for BufferRef {
    fn drop(&mut self) {
        // SAFETY: `drop` will only be called once
        let buffer = unsafe { ManuallyDrop::take(&mut self.buffer) };
        // If driver is dropped, release the buffer.
        if let Some(shared) = self.shared.upgrade() {
            shared.reset(self.buffer_id, buffer);
        }
    }
}
#[cfg(any(fusion, not(io_uring)))]
pub(crate) mod fallback {
    use std::{collections::VecDeque, io, mem::MaybeUninit};

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

        pub unsafe fn reset(&mut self, buffer_id: u16, _: &[MaybeUninit<u8>]) {
            self.queue.push_back(buffer_id);
        }
    }
}
