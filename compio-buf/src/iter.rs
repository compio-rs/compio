use crate::*;

/// The inner implementation of a [`OwnedBufIterator`].
pub trait OwnedIterator: IntoInner + Sized {
    /// Get the next iterator. Will return Err with the inner buffer if it
    /// reaches the end.
    fn next(self) -> Result<Self, Self::Inner>;

    /// Get the current buffer.
    fn current(&self) -> &dyn IoBuf;
}

/// The mutable part of inner implementation of a [`OwnedBufIterator`].
pub trait OwnedIteratorMut: OwnedIterator {
    /// Get the current mutable buffer.
    fn current_mut(&mut self) -> &mut dyn IoBufMut;
}

/// An owned buffer iterator for vectored buffers.
/// See [`IoVectoredBuf::owned_iter`].
#[derive(Debug)]
pub struct OwnedIter<I: OwnedIterator>(I);

impl<I: OwnedIterator> OwnedIter<I> {
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

impl<I: OwnedIterator> IntoInner for OwnedIter<I> {
    type Inner = I::Inner;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}

unsafe impl<I: OwnedIterator + Unpin + 'static> IoBuf for OwnedIter<I> {
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

unsafe impl<I: OwnedIteratorMut + Unpin + 'static> IoBufMut for OwnedIter<I> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.0.current_mut().as_buf_mut_ptr()
    }
}

impl<I: OwnedIteratorMut + Unpin + 'static> SetBufInit for OwnedIter<I> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.0.current_mut().set_buf_init(len)
    }
}

/// An owned buffer iterator for vectored buffers.
/// See [`IoVectoredBuf::owned_iter`].
pub(crate) struct IndexedIter<T> {
    bufs: T,
    nth: usize,
}

impl<T: IoIndexedBuf> IndexedIter<T> {
    pub(crate) fn new(bufs: T, nth: usize) -> Result<Self, T> {
        if bufs.buf_nth(nth).is_none() {
            Err(bufs)
        } else {
            Ok(Self { bufs, nth })
        }
    }
}

impl<T: IoIndexedBuf> OwnedIterator for IndexedIter<T> {
    fn next(self) -> Result<Self, Self::Inner> {
        Self::new(self.bufs, self.nth + 1)
    }

    fn current(&self) -> &dyn IoBuf {
        self.bufs
            .buf_nth(self.nth)
            .expect("the nth buf should exist")
    }
}

impl<T: IoIndexedBufMut> OwnedIteratorMut for IndexedIter<T> {
    fn current_mut(&mut self) -> &mut dyn IoBufMut {
        self.bufs
            .buf_nth_mut(self.nth)
            .expect("the nth buf should exist")
    }
}

impl<T> IntoInner for IndexedIter<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.bufs
    }
}
