use std::{fmt::Debug, mem::MaybeUninit,slice};

/// An unsafely `'static` and platform-agnostic buffer that's initialized.
///
/// It's structurally equal to `&[u8]` but with `'static` lifetime, making
/// interaction with the runtime easier.
#[derive(Clone, Copy)]
pub struct IoBuffer {
    ptr: *const u8,
    len: usize,
}

impl Debug for IoBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe { self.slice() }.fmt(f)
    }
}

impl IoBuffer {
    /// Create a new [`IoBuffer`] from a raw pointer and a length.
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the pointer points to a valid memory region of at least `len` bytes
    /// - the pointer is valid for the lifetime of the [`IoBuffer`]
    /// - data within range is initialized
    /// - data within range is not being mutated while the [`IoBuffer`] is in
    ///   use.
    pub unsafe fn new(ptr: *const u8, len: usize) -> Self {
        Self { len, ptr }
    }

    /// Create a new [`IoBuffer`] from an initialized slice.
    ///
    /// # Safety
    /// Data within the range must not be mutated while the [`IoBuffer`] is in
    /// use, and returned [`IoBuffer`] must not outlive `slice`.
    pub unsafe fn from_slice(slice: &[u8]) -> Self {
        Self {
            ptr: slice.as_ptr(),
            len: slice.len(),
        }
    }

    /// Create a slice from the buffer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the [`IoBuffer`] is valid during `'a`.
    pub unsafe fn slice<'a>(self) -> &'a [u8] {
        // SAFETY: (by type invariant)
        // - the length is correct
        // - data within range is initialized
        // - data within range is not being mutated while the [`IoBuffer`] is in use.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Decompose the [`IoBuffer`] into pointer and length.
    pub fn into_piece(self) -> (*const u8, usize) {
        (self.ptr, self.len)
    }

    /// Get the pointer to the buffer.
    pub fn as_ptr(&self) -> *const u8 {
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
}

/// An unsafely `'static`, platform-agnostic buffer that may be uninitialized.
///
/// It's structurally equal to `&mut [MaybeUninit<u8>]` but with `'static`
/// lifetime, making interaction with the runtime easier.
pub struct IoBufferMut {
    ptr: *mut MaybeUninit<u8>,
    len: usize,
}

impl Debug for IoBufferMut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe { slice::from_raw_parts::<MaybeUninit<u8>>(self.ptr as *const _, self.len) }.fmt(f)
    }
}

impl IoBufferMut {
    /// Create a new [`IoBufferMut`] from a raw pointer and a length
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - the pointer points to a valid memory region of at least `len`
    ///   uninitialized bytes
    /// - the pointer is valid for the lifetime of the [`IoBufferMut`]
    /// - the length is correct (the content can be uninitialized, but must be
    ///   accessible)
    /// - the pointer is not used for anything else while the [`IoBufferMut`] is
    ///   in use
    pub unsafe fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
        Self { len, ptr }
    }

    /// Create a new [`IoBufferMut`] from an initialized slice.
    ///
    /// # Safety
    /// Data within the range must not be used while the [`IoBufferMut`] is
    /// in use, and returned [`IoBufferMut`] must not outlive `slice`.
    pub unsafe fn from_slice(slice: &mut [MaybeUninit<u8>]) -> Self {
        Self {
            ptr: slice.as_mut_ptr(),
            len: slice.len(),
        }
    }

    /// Create a new [`IoBufferMut`] from an uninitialized slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the slice is valid the and is not used for
    /// anything else during `'a`.
    pub unsafe fn slice_mut<'a>(self) -> &'a mut [MaybeUninit<u8>] {
        // SAFETY: (by type invariant)
        // - the length is correct
        // - the slice is not used for anything else while the [`IoBufferMut`] is in use
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// Decompose the [`IoBufferMut`] into pointer and length.
    pub fn into_piece(self) -> (*mut MaybeUninit<u8>, usize) {
        (self.ptr, self.len)
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
}
