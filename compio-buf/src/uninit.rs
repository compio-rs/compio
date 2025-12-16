use std::mem::MaybeUninit;

use crate::*;

/// A [`Slice`] that only exposes uninitialized bytes.
///
/// [`Uninit`] can be created with [`IoBufMut::uninit`].
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

impl<T: IoBuf> IoBuf for Uninit<T> {
    fn as_slice(&self) -> &[u8] {
        self.0.as_slice() // this is always &[] but we can't return &[] since the pointer will be different
    }
}

impl<T: IoBufMut> IoBufMut for Uninit<T> {
    fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
        let len = (*self).buf_len();
        &mut self.0.as_uninit()[len..]
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
