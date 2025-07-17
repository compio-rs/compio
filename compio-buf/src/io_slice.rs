use std::mem::MaybeUninit;

#[cfg(unix)]
mod sys {
    use std::mem::MaybeUninit;

    #[repr(transparent)]
    pub struct Inner(libc::iovec);

    impl Inner {
        pub fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
            Self(libc::iovec {
                iov_base: ptr as *mut libc::c_void,
                iov_len: len,
            })
        }

        pub fn len(&self) -> usize {
            self.0.iov_len
        }

        pub fn as_ptr(&self) -> *mut MaybeUninit<u8> {
            self.0.iov_base as *mut MaybeUninit<u8>
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::mem::MaybeUninit;

    // Copied from std
    #[repr(C)]
    #[allow(clippy::upper_case_acronyms)]
    struct WSABUF {
        pub len: u32,
        pub buf: *mut MaybeUninit<u8>,
    }

    #[repr(transparent)]
    pub struct Inner(WSABUF);

    impl Inner {
        pub fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
            Self(WSABUF {
                len: len as u32,
                buf: ptr,
            })
        }

        pub fn len(&self) -> usize {
            self.0.len as _
        }

        pub fn as_ptr(&self) -> *mut MaybeUninit<u8> {
            self.0.buf
        }
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("`IoSlice` only available on unix and windows");

/// An unsafe, `'static`, initialized, and immutable slice of bytes to interact
/// with system API.
///
/// Like [`IoSlice`] in `std`, `IoSlice` guarantees the ABI compatibility
/// on unix and windows, but without the lifetime, makes it easier to use with
/// compio driver at the cost of unsafe to construct. `IoSlice` should only be
/// used with compio driver.
///
/// [`IoSlice`]: std::io::IoSlice
#[repr(transparent)]
pub struct IoSlice(sys::Inner);

impl IoSlice {
    /// Create a new `IoSlice` from a raw pointer and a length.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the pointer is valid for the lifetime of the `IoSlice`
    /// - the length is correct
    /// - the content of the buffer is initialized
    /// - the pointer is not used for mutating while the `IoSlice` is in use
    pub unsafe fn new(ptr: *const u8, len: usize) -> Self {
        Self(sys::Inner::new(ptr as _, len))
    }

    /// Create a new `IoSlice` from an initialized slice.
    ///
    /// # Safety
    /// The caller must ensure that, during the lifetime of the `IoSlice`, the
    /// slice is valid the and is not used for mutating.
    pub unsafe fn from_slice(slice: &[u8]) -> Self {
        Self::new(slice.as_ptr() as _, slice.len())
    }

    /// Get the pointer to the buffer.
    pub fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr() as _
    }

    /// Get the length of the buffer.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An unsafe, `'static`, maybe uninitialized, and mutable slice of bytes to
/// interact with system API.
///
/// Like [`IoSliceMut`] in `std`, `IoSliceMut` guarantees the ABI compatibility
/// on unix and windows, but without the lifetime and accepts
/// [`MaybeUninit<u8>`], makes it easier to use with compio driver at the cost
/// of unsafe to construct. `IoSliceMut` should only be used with compio driver.
///
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub struct IoSliceMut(sys::Inner);

impl IoSliceMut {
    /// Create a new `IoSliceMut` from a raw pointer and a length.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the pointer is valid for the lifetime of the `IoSliceMut`
    /// - the length is correct (the content can be uninitialized, but must be
    ///   accessible)
    /// - the pointer is not used for anything else while the `IoSliceMut` is in
    ///   use
    pub unsafe fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
        Self(sys::Inner::new(ptr, len))
    }

    /// Create a new `IoSliceMut` from an initialized slice.
    ///
    /// # Safety
    /// The caller must ensure that, during the lifetime of the `IoSliceMut`,
    /// the slice is valid the and is not used for anything else.
    pub unsafe fn from_slice(slice: &mut [u8]) -> Self {
        Self::new(slice.as_mut_ptr() as _, slice.len())
    }

    /// Create a new `IoSliceMut` from a uninitialized slice.
    ///
    /// # Safety
    /// The caller must ensure that, during the lifetime of the `IoSliceMut`,
    /// the slice is valid the and is not used for anything else.
    pub unsafe fn from_uninit(slice: &mut [MaybeUninit<u8>]) -> Self {
        Self::new(slice.as_mut_ptr(), slice.len())
    }

    /// Get the pointer to the buffer.
    pub fn as_ptr(&self) -> *mut MaybeUninit<u8> {
        self.0.as_ptr()
    }

    /// Get the length of the buffer.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An unsafe, `'static`, partially initialized, and mutable buffer.
///
/// It contains full information to describe the buffer.
pub struct IoBuffer {
    len: usize,
    capacity: usize,
    ptr: *mut MaybeUninit<u8>,
}

impl IoBuffer {
    /// Create a new [`IoBuffer`] from a raw pointer, a length, and a capacity.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the pointer is valid for the lifetime of the `IoBuffer`
    /// - the length is correct (the content can be uninitialized, but must be
    ///   accessible)
    /// - The capacity should not be smaller than the length.
    /// - the pointer is not used for anything else while the `IoBuffer` is in
    ///   use
    pub unsafe fn new(ptr: *mut MaybeUninit<u8>, len: usize, capacity: usize) -> Self {
        Self { len, capacity, ptr }
    }

    /// Get the pointer to the buffer.
    pub fn as_ptr(&self) -> *mut MaybeUninit<u8> {
        self.ptr
    }

    /// Get the initialized length of the buffer.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the full capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl From<IoBuffer> for IoSlice {
    fn from(value: IoBuffer) -> Self {
        unsafe { Self::new(value.ptr.cast(), value.len) }
    }
}

impl From<IoBuffer> for IoSliceMut {
    fn from(value: IoBuffer) -> Self {
        unsafe { Self::new(value.ptr.cast(), value.capacity) }
    }
}
