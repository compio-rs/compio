use std::ops::{Deref, DerefMut};

use crate::*;

/// An [`Slice`] that only exposes uninitialized bytes.
///
/// [`Uninit`] can be created with [`IoBuf::uninit`].
///
/// # Examples
///
/// Creating an uninit slice
///
/// ```
/// use compio_buf::IoBuf;
///
/// let buf = b"hello world";
/// let slice = buf.uninit();
///
/// assert_eq!(&slice[..], b"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uninit<T>(Slice<T>);

impl<T: IoBuf> Uninit<T> {
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

impl<T: IoBuf> Deref for Uninit<T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: IoBufMut> DerefMut for Uninit<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

unsafe impl<T: IoBuf> IoBuf for Uninit<T> {
    fn as_buf_ptr(&self) -> *const u8 {
        self.0.as_buf_ptr()
    }

    fn buf_len(&self) -> usize {
        self.0.buf_len()
    }

    fn buf_capacity(&self) -> usize {
        self.0.buf_capacity()
    }
}

unsafe impl<T: IoBufMut> IoBufMut for Uninit<T> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_buf_mut_ptr()
    }
}

impl<T: SetBufInit> SetBufInit for Uninit<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.0.set_buf_init(len);
    }
}

impl<T> IntoInner for Uninit<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}
