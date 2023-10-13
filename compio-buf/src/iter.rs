use crate::*;

/// The inner implementation of a [`OwnedBufIterator`].
pub trait OwnedBufIteratorInner: IntoInner + Sized {
    /// Get the next iterator. Will return Err with the inner buffer if it
    /// reaches the end.
    fn next(self) -> Result<Self, Self::Inner>;

    /// Get the current buffer.
    fn current(&self) -> &dyn IoBuf;
}

/// The mutable part of inner implementation of a [`OwnedBufIterator`].
pub trait OwnedBufIteratorInnerMut: OwnedBufIteratorInner {
    /// Get the current mutable buffer.
    fn current_mut(&mut self) -> &mut dyn IoBufMut;
}

/// An owned buffer iterator for vectored buffers.
/// See [`IoVectoredBuf::owned_iter`].
#[derive(Debug)]
pub struct OwnedBufIterator<I: OwnedBufIteratorInner>(I);

impl<I: OwnedBufIteratorInner> OwnedBufIterator<I> {
    /// Create [`OwnedBufIterator`] from inner impls.
    pub fn new(inner: I) -> Self {
        Self(inner)
    }

    /// Get the next buffer. Will return Err with the inner buffer if it reaches
    /// the end.
    pub fn next(self) -> Result<Self, I::Inner> {
        self.0.next().map(Self::new)
    }
}

impl<I: OwnedBufIteratorInner> IntoInner for OwnedBufIterator<I> {
    type Inner = I::Inner;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}

unsafe impl<I: OwnedBufIteratorInner + Unpin + 'static> IoBuf for OwnedBufIterator<I> {
    fn as_buf_ptr(&self) -> *const u8 {
        self.0.current().as_buf_ptr()
    }

    fn buf_len(&self) -> usize {
        self.0.current().buf_len()
    }

    fn buf_capacity(&self) -> usize {
        self.0.current().buf_capacity()
    }
}

unsafe impl<I: OwnedBufIteratorInnerMut + Unpin + 'static> IoBufMut for OwnedBufIterator<I> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.0.current_mut().as_buf_mut_ptr()
    }
}

impl<I: OwnedBufIteratorInnerMut + Unpin + 'static> SetBufInit for OwnedBufIterator<I> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.0.current_mut().set_buf_init(len)
    }
}

/// An owned buffer iterator for vectored buffers.
/// See [`IoVectoredBuf::owned_iter`].
pub(crate) struct IndexedBufIterInner<T> {
    bufs: T,
    nth: usize,
}

impl<T: IoIndexedBuf> IndexedBufIterInner<T> {
    pub(crate) fn new(bufs: T, nth: usize) -> Result<Self, T> {
        if bufs.buf_nth(nth).is_none() {
            Err(bufs)
        } else {
            Ok(Self { bufs, nth })
        }
    }
}

impl<T: IoIndexedBuf> OwnedBufIteratorInner for IndexedBufIterInner<T> {
    fn next(self) -> Result<Self, Self::Inner> {
        Self::new(self.bufs, self.nth + 1)
    }

    fn current(&self) -> &dyn IoBuf {
        self.bufs
            .buf_nth(self.nth)
            .expect("the nth buf should exist")
    }
}

impl<T: IoIndexedBufMut> OwnedBufIteratorInnerMut for IndexedBufIterInner<T> {
    fn current_mut(&mut self) -> &mut dyn IoBufMut {
        self.bufs
            .buf_nth_mut(self.nth)
            .expect("the nth buf should exist")
    }
}

impl<T> IntoInner for IndexedBufIterInner<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.bufs
    }
}
