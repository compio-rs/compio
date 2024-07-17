//! Buffer pool

use std::{io, marker::PhantomData, mem::ManuallyDrop};

use crate::Runtime;

/// Buffer borrowed from buffer pool
///
/// When IO operation finish, user will obtain a `BorrowedBuffer` to access the
/// filled data
pub type BorrowedBuffer<'a> = compio_driver::BorrowedBuffer<'a>;

/// Buffer pool
///
/// A buffer pool to allow user no need to specify a specific buffer to do the
/// IO operation
///
/// Drop the `BufferPool` will release the buffer pool automatically
#[derive(Debug)]
pub struct BufferPool {
    inner: ManuallyDrop<compio_driver::BufferPool>,
    runtime_id: i64,

    // make it !Send and !Sync, to prevent user send the buffer pool to other thread
    _marker: PhantomData<*const ()>,
}

impl BufferPool {
    /// Create buffer pool with given `buffer_size` and `buffer_len`
    ///
    /// # Notes
    ///
    /// If `buffer_len` is not power of 2, it will be upward with
    /// [`u16::next_power_of_two`]
    pub fn new(buffer_len: u16, buffer_size: usize) -> io::Result<Self> {
        let (inner, runtime_id) = Runtime::with_current(|runtime| {
            let buffer_pool = runtime.create_buffer_pool(buffer_len, buffer_size)?;
            let runtime_id = runtime.id();

            Ok((buffer_pool, runtime_id))
        })?;

        Ok(Self::inner_new(inner, runtime_id))
    }

    fn inner_new(inner: compio_driver::BufferPool, runtime_id: i64) -> Self {
        Self {
            inner: ManuallyDrop::new(inner),
            runtime_id,
            _marker: Default::default(),
        }
    }

    /// Get the inner driver buffer pool reference
    ///
    /// # Notes
    ///
    /// You should not use this method unless you are writing your own IO opcode
    ///
    /// # Panic
    ///
    /// If call this method in incorrect runtime, will panic
    pub fn as_driver_buffer_pool(&self) -> &compio_driver::BufferPool {
        let current_runtime_id = Runtime::with_current(|runtime| runtime.id());
        assert_eq!(current_runtime_id, self.runtime_id);

        &self.inner
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        let _ = Runtime::try_with_current(|runtime| {
            if self.runtime_id != runtime.id() {
                return;
            }

            unsafe {
                // Safety: we own the inner
                let inner = ManuallyDrop::take(&mut self.inner);

                // Safety: the buffer pool is created by current thread runtime
                let _ = runtime.release_buffer_pool(inner);
            }
        });
    }
}
