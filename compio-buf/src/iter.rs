use crate::*;

/// The inner implementation of a [`OwnedIter`].
pub trait OwnedIterator: IntoInner + Sized {
    /// Get the next iterator.
    ///
    /// If current `Self` is the last one, return `Err(Self)` with `Self` being
    /// untouched.
    fn next(self) -> Result<Self, Self::Inner>;
}

/// An owned iterator over an indexable container.
pub struct IndexedIter<T> {
    items: T,
    nth: usize,
}

impl<T: Indexable> IndexedIter<T> {
    /// Create a new [`IndexedIter`] from an indexable container. If the
    /// container is empty, return the buffer back in `Err(T)`.
    pub fn new(bufs: T) -> Result<Self, T> {
        if bufs.index(0).is_none() {
            Err(bufs)
        } else {
            Ok(Self {
                items: bufs,
                nth: 0,
            })
        }
    }
}

unsafe impl<T> IoBuf for IndexedIter<T>
where
    T: Indexable + 'static,
    T::Output: IoBuf,
{
    fn as_buf_ptr(&self) -> *const u8 {
        self.items.index(self.nth).unwrap().as_buf_ptr()
    }

    fn buf_len(&self) -> usize {
        self.items.index(self.nth).unwrap().buf_len()
    }

    fn buf_capacity(&self) -> usize {
        self.items.index(self.nth).unwrap().buf_capacity()
    }
}

impl<T> SetBufInit for IndexedIter<T>
where
    T: IndexableMut,
    T::Output: IoBufMut,
{
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.items.index_mut(self.nth).unwrap().set_buf_init(len)
    }
}

unsafe impl<T> IoBufMut for IndexedIter<T>
where
    T: IndexableMut + 'static,
    T::Output: IoBufMut,
{
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.items.index_mut(self.nth).unwrap().as_buf_mut_ptr()
    }
}

impl<T> IntoInner for IndexedIter<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.items
    }
}

impl<T: Indexable> OwnedIterator for IndexedIter<T> {
    fn next(self) -> Result<Self, Self::Inner> {
        if self.items.index(self.nth + 1).is_some() {
            Ok(Self {
                items: self.items,
                nth: self.nth + 1,
            })
        } else {
            Err(self.into_inner())
        }
    }
}

/// A trait for vectored buffers that could be indexed.
pub trait Indexable {
    /// Output item
    type Output;

    /// Get the item with specific index.
    fn index(&self, n: usize) -> Option<&Self::Output>;
}

/// A trait for vectored buffers that could be mutably indexed.
pub trait IndexableMut: Indexable {
    /// Get the mutable item with specific index.
    fn index_mut(&mut self, n: usize) -> Option<&mut Self::Output>;
}

impl<T> Indexable for &[T] {
    type Output = T;

    fn index(&self, n: usize) -> Option<&T> {
        self.get(n)
    }
}

impl<T> Indexable for &mut [T] {
    type Output = T;

    fn index(&self, n: usize) -> Option<&T> {
        self.get(n)
    }
}

impl<T: Indexable> Indexable for &T {
    type Output = T::Output;

    fn index(&self, n: usize) -> Option<&T::Output> {
        (**self).index(n)
    }
}

impl<T: Indexable> Indexable for &mut T {
    type Output = T::Output;

    fn index(&self, n: usize) -> Option<&T::Output> {
        (**self).index(n)
    }
}

impl<T, const N: usize> Indexable for [T; N] {
    type Output = T;

    fn index(&self, n: usize) -> Option<&T> {
        self.get(n)
    }
}

impl<T, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static> Indexable
    for t_alloc!(Vec, T, A)
{
    type Output = T;

    fn index(&self, n: usize) -> Option<&T> {
        self.get(n)
    }
}

#[cfg(feature = "arrayvec")]
impl<T, const N: usize> Indexable for arrayvec::ArrayVec<T, N> {
    type Output = T;

    fn index(&self, n: usize) -> Option<&T> {
        self.get(n)
    }
}

impl<T> IndexableMut for &mut [T] {
    fn index_mut(&mut self, n: usize) -> Option<&mut T> {
        self.get_mut(n)
    }
}

impl<T: IndexableMut> IndexableMut for &mut T {
    fn index_mut(&mut self, n: usize) -> Option<&mut T::Output> {
        (**self).index_mut(n)
    }
}

impl<T, const N: usize> IndexableMut for [T; N] {
    fn index_mut(&mut self, n: usize) -> Option<&mut T> {
        self.get_mut(n)
    }
}

impl<T, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static> IndexableMut
    for t_alloc!(Vec, T, A)
{
    fn index_mut(&mut self, n: usize) -> Option<&mut T> {
        self.get_mut(n)
    }
}

#[cfg(feature = "arrayvec")]
impl<T, const N: usize> IndexableMut for arrayvec::ArrayVec<T, N> {
    fn index_mut(&mut self, n: usize) -> Option<&mut T> {
        self.get_mut(n)
    }
}
