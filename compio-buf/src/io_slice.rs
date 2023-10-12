use std::mem::MaybeUninit;

#[cfg(unix)]
mod sys {
    #[repr(transparent)]
    pub struct Inner(libc::iovec);

    impl Inner {
        pub fn new(ptr: *mut u8, len: usize) -> Self {
            Self(libc::iovec {
                iov_base: ptr as *mut libc::c_void,
                iov_len: len,
            })
        }

        pub fn len(&self) -> usize {
            self.0.iov_len
        }

        pub fn as_ptr(&self) -> *mut u8 {
            self.0.iov_base as *mut u8
        }
    }
}

#[cfg(windows)]
mod sys {
    // Copied from std
    #[repr(C)]
    struct WSABUF {
        pub len: u32,
        pub buf: *mut u8,
    }

    #[repr(transparent)]
    pub struct Inner(WSABUF);

    impl Inner {
        pub fn new(ptr: *mut u8, len: usize) -> Self {
            Self(WSABUF {
                len: len as u32,
                buf: ptr,
            })
        }

        pub fn len(&self) -> usize {
            self.0.len as _
        }

        pub fn as_ptr(&self) -> *mut u8 {
            self.0.buf
        }
    }
}

/// An unsafe `'static` slice of bytes to interact with os api.
///
/// Like [`IoSliceMut`] in `std`, `IoSlice` guarantees the ABI compatability
/// on unix and windows, but without the lifetime, makes it easier to use with
/// compio driver as they need to take ownership (`'static`) of the
/// buffer. However, this does makes the type unsafe to construct, and should
/// only be used with compio driver.
///
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub struct IoSlice(sys::Inner);

impl IoSlice {
    /// Create a new `IoSlice` from a raw pointer and a length.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the pointer is valid for the lifetime of the `IoSlice`
    /// - the length is correct (the content can be uninitialized, but must be
    ///   accessible)
    /// - the pointer is not used for anything else while the `IoSlice` is in
    ///   use
    pub unsafe fn new(ptr: *mut u8, len: usize) -> Self {
        Self(sys::Inner::new(ptr, len))
    }

    /// Create a new `IoSlice` from an initialized slice.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the slice is valid for the lifetime of the `IoSlice`
    /// - the slice is not used for anything else while the `IoSlice` is in use
    pub unsafe fn from_slice(slice: &mut [u8]) -> Self {
        Self::new(slice.as_mut_ptr(), slice.len())
    }

    /// Create a new `IoSlice` from a uninitialized slice.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the slice is valid for the lifetime of the `IoSlice`
    /// - the slice is not used for anything else while the `IoSlice` is in use
    pub unsafe fn from_uninit(slice: &mut [MaybeUninit<u8>]) -> Self {
        Self::new(slice.as_mut_ptr() as *mut u8, slice.len())
    }

    /// Break the `IoSlice` into a raw pointer and a length.
    pub fn into_parts(self) -> (*mut u8, usize) {
        (self.0.as_ptr(), self.0.len())
    }

    /// Get the mutable pointer to the buffer.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.0.as_ptr()
    }

    /// Get the pointer to the buffer.
    pub fn as_ptr(&self) -> *const u8 {
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
