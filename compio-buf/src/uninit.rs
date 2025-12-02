use std::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    slice,
};

use crate::*;

/// A [`Slice`] that only exposes uninitialized bytes.
///
/// [`Uninit`] can be created with [`IoBuf::uninit`].
///
/// # Examples
///
/// Creating an uninit slice
///
/// ```
/// use compio_buf::{IoBuf, IoBufMut};
///
/// let mut buf = Vec::from(b"hello world");
/// buf.reserve_exact(10);
/// let slice = buf.uninit();
///
/// println!("{}", slice.uninit_len());
/// assert_eq!(slice.uninit_len(), 10);
/// assert_eq!(slice.as_slice(), b"");
/// assert_eq!(slice.buf_capacity(), 10);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uninit<T>(Slice<T>);

impl<T: IoBufMut> Uninit<T> {
    pub(crate) fn new(buffer: T) -> Self {
        let len = buffer.buf_len();
        Self(buffer.slice(len..))
    }
}

impl<T> Uninit<T> {
    /// Offset in the underlying buffer at which uninitialized bytes starts.
    pub fn begin(&self) -> usize {
        self.0.begin()
    }

    /// Gets a reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner(&self) -> &T {
        self.0.as_inner()
    }

    /// Gets a mutable reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn as_inner_mut(&mut self) -> &mut T {
        self.0.as_inner_mut()
    }
}

impl<T: IoBufMut> Deref for Uninit<T> {
    type Target = [MaybeUninit<u8>];

    fn deref(&self) -> &Self::Target {
        let ptr = unsafe { self.0.as_ptr().add(self.begin()) as *const MaybeUninit<u8> };
        let len = self.0.uninit_len();
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

impl<T: IoBufMut> DerefMut for Uninit<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let ptr = unsafe { self.0.as_ptr().add(self.begin()) as *mut MaybeUninit<u8> };
        let len = self.0.uninit_len();
        unsafe { slice::from_raw_parts_mut(ptr, len) }
    }
}

unsafe impl<T: IoBuf> IoBuf for Uninit<T> {
    unsafe fn buffer(&self) -> IoBuffer {
        // SAFETY: we are just forwarding the call
        unsafe { self.0.buffer() }
    }
}

unsafe impl<T: IoBufMut> IoBufMut for Uninit<T> {
    fn uninit_len(&self) -> usize {
        self.0.uninit_len()
    }
}

impl<T: SetBufInit + IoBuf> SetBufInit for Uninit<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        unsafe {
            self.0.set_buf_init(len);
        }
    }
}

impl<T> IntoInner for Uninit<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}
