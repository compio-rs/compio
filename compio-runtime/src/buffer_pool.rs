#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub use io_uring::BufferPool;

pub type BorrowedBuffer<'a> = compio_driver::BorrowedBuffer<'a>;

#[cfg(not(feature = "io-uring"))]
pub use fallback::BufferPool;

#[cfg(not(feature = "io-uring"))]
mod fallback {
    use std::{io, marker::PhantomData};

    use crate::Runtime;

    #[derive(Debug)]
    pub struct BufferPool {
        inner: compio_driver::BufferPool,

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
            Runtime::with_current(|runtime| {
                runtime.create_buffer_pool(buffer_len.next_power_of_two(), buffer_size)
            })
        }

        pub(crate) fn inner_new(inner: compio_driver::BufferPool) -> Self {
            Self {
                inner,
                _marker: Default::default(),
            }
        }

        // user should not use this method, but compio_net and compio_fs need it to
        // access the driver buffer pool, so make it public and hide it from doc
        #[doc(hidden)]
        pub fn as_driver_buffer_pool(&self) -> &compio_driver::BufferPool {
            &self.inner
        }

        // user should not use this method, but compio_net and compio_fs need it to
        // access the driver buffer pool, so make it public and hide it from doc
        #[doc(hidden)]
        pub fn get_buffer(&self) -> Option<Vec<u8>> {
            self.inner.get_buffer()
        }

        // user should not use this method, but compio_net and compio_fs need it to
        // access the driver buffer pool, so make it public and hide it from doc
        #[doc(hidden)]
        pub fn add_buffer(&self, buffer: Vec<u8>) {
            self.inner.add_buffer(buffer)
        }
    }
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod io_uring {
    use std::{io, marker::PhantomData, mem::ManuallyDrop};

    use super::BorrowedBuffer;
    use crate::Runtime;

    #[derive(Debug)]
    pub struct BufferPool {
        inner: ManuallyDrop<compio_driver::BufferPool>,

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
            Runtime::with_current(|runtime| runtime.create_buffer_pool(buffer_len, buffer_size))
        }

        pub(crate) fn inner_new(inner: compio_driver::BufferPool) -> Self {
            Self {
                inner: ManuallyDrop::new(inner),
                _marker: Default::default(),
            }
        }

        // user should not use this method, but compio_net and compio_fs need it to
        // access the driver buffer pool, so make it public and hide it from doc
        #[doc(hidden)]
        pub fn as_driver_buffer_pool(&self) -> &compio_driver::BufferPool {
            &self.inner
        }

        // user should not use this method, but compio_net and compio_fs need it to
        // access the driver buffer pool, so make it public and hide it from doc
        #[doc(hidden)]
        pub unsafe fn get_buffer(
            &self,
            flags: u32,
            available_len: usize,
        ) -> Option<BorrowedBuffer> {
            self.inner.get_buffer(flags, available_len)
        }
    }

    impl Drop for BufferPool {
        fn drop(&mut self) {
            let _ = Runtime::try_with_current(|runtime| {
                unsafe {
                    // Safety: we own the inner
                    let inner = ManuallyDrop::take(&mut self.inner);

                    // Safety: the buffer pool is created by current thread runtime
                    let _ = runtime.release_buffer_pool(inner);
                }
            });
        }
    }
}
