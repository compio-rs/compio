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
/// use compio_buf::IoBuf;
///
/// let mut buf = Vec::from(b"hello world");
/// buf.reserve_exact(10);
/// let slice = buf.uninit();
///
/// assert_eq!(slice.as_slice(), b"");
/// assert_eq!(slice.buf_capacity(), 10);
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

unsafe impl<T: IoBuf> IoBuf for Uninit<T> {
    fn as_buf_ptr(&self) -> *const u8 {
        self.0.as_buf_ptr()
    }

    fn buf_len(&self) -> usize {
        debug_assert!(self.0.buf_len() == 0, "Uninit buffer should have length 0");
        0
    }

    fn buf_capacity(&self) -> usize {
        self.0.buf_capacity()
    }

    fn as_slice(&self) -> &[u8] {
        &[]
    }
}

unsafe impl<T: IoBufMut> IoBufMut for Uninit<T> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_buf_mut_ptr()
    }
}

impl<T: SetBufInit + IoBuf> SetBufInit for Uninit<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.0.set_buf_init(self.0.buf_len() + len);
        let inner = self.0.as_inner();
        self.0.set_range(inner.buf_len(), inner.buf_capacity());
    }
}

impl<T> IntoInner for Uninit<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}
