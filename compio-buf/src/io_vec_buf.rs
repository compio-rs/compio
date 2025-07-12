use std::{cmp::min, ops::{Deref, DerefMut}};

use crate::{
    IndexedIter, IoBuf, IoBufMut, IoSlice, IoSliceMut, OwnedIterator, SetBufInit, t_alloc,
};

/// A type that's either owned or borrowed. Like [`Cow`](std::borrow::Cow) but
/// without the requirement of [`ToOwned`].
pub enum MaybeOwned<'a, T> {
    /// Owned.
    Owned(T),
    /// Borrowed.
    Borrowed(&'a T),
}

impl<T> Deref for MaybeOwned<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            MaybeOwned::Owned(t) => t,
            MaybeOwned::Borrowed(t) => t,
        }
    }
}

/// A type that's either owned or mutably borrowed .
pub enum MaybeOwnedMut<'a, T> {
    /// Owned.
    Owned(T),
    /// Mutably borrowed.
    Borrowed(&'a mut T),
}

impl<T> Deref for MaybeOwnedMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            MaybeOwnedMut::Owned(t) => t,
            MaybeOwnedMut::Borrowed(t) => t,
        }
    }
}

impl<T> DerefMut for MaybeOwnedMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            MaybeOwnedMut::Owned(t) => t,
            MaybeOwnedMut::Borrowed(t) => t,
        }
    }
}

/// A trait for homogeneous vectored buffers.
pub trait IoVectoredBuf: 'static {
    /// The buffer.
    type Buf: IoBuf;

    /// The owned iterator that wraps `Self`.
    type OwnedIter: OwnedIterator<Inner = Self> + IoBuf;

    /// Collected [`IoSlice`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn io_slices(&self) -> Vec<IoSlice> {
        self.iter_buf().map(|buf| buf.as_io_slice()).collect()
    }

    /// An iterator over the buffers that's either owned or borrowed with
    /// [`MaybeOwned`].
    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, Self::Buf>>;

    /// Wrap self into an owned iterator.
    fn owned_iter(self) -> Result<Self::OwnedIter, Self>
    where
        Self: Sized;
}

impl<T: IoBuf> IoVectoredBuf for &'static [T] {
    type Buf = T;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, T>> {
        self.iter().map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self> {
        IndexedIter::new(self)
    }
}

impl<T: IoBuf> IoVectoredBuf for &'static mut [T] {
    type Buf = T;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, T>> {
        self.iter().map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self> {
        IndexedIter::new(self)
    }
}

impl<T: IoBuf, const N: usize> IoVectoredBuf for [T; N] {
    type Buf = T;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, T>> {
        self.iter().map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self> {
        IndexedIter::new(self)
    }
}

impl<T: IoBuf, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static> IoVectoredBuf
    for t_alloc!(Vec, T, A)
{
    type Buf = T;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, T>> {
        self.iter().map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self> {
        IndexedIter::new(self)
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for arrayvec::ArrayVec<T, N> {
    type Buf = T;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, T>> {
        self.iter().map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self> {
        IndexedIter::new(self)
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBuf, const N: usize> IoVectoredBuf for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    type Buf = T;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, T>> {
        self.iter().map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self> {
        IndexedIter::new(self)
    }
}

/// A trait for heterogeneous vectored buffers.
pub trait IoVectoredBuf2: 'static {

    /// Collected [`IoSlice`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn io_slices(&self) -> Vec<IoSlice> {
        self.iter_ioslice().collect()
    }

    /// An iterator over the [`IoSlice`]s.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn iter_ioslice(&self) -> impl Iterator<Item = IoSlice>;
}

impl<T: IoBuf, Rest: IoVectoredBuf2> IoVectoredBuf2 for (T, Rest) {
    unsafe fn iter_ioslice(&self) -> impl Iterator<Item = IoSlice> {
        std::iter::once(self.0.as_io_slice()).chain(self.1.iter_ioslice())
    }
}

impl IoVectoredBuf2 for () {
    unsafe fn iter_ioslice(&self) -> impl Iterator<Item = IoSlice> {
        std::iter::empty()
    }
}

impl<T: IoVectoredBuf> IoVectoredBuf2 for T {
    unsafe fn iter_ioslice(&self) -> impl Iterator<Item = IoSlice> {
        self.iter_buf().map(|buf| buf.as_io_slice())
    }
}

/// A trait for mutable homogeneous vectored buffers.
pub trait IoVectoredBufMut: IoVectoredBuf<Buf: IoBufMut, OwnedIter: IoBufMut> + SetBufInit {
    /// An iterator for the [`IoSliceMut`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn io_slices_mut(&mut self) -> Vec<IoSliceMut> {
        self.iter_buf_mut()
            .map(|mut buf| buf.as_io_slice_mut())
            .collect()
    }

    /// An iterator over the buffers that's either owned or mutably borrowed
    /// with [`MaybeOwnedMut`].
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>>;
}

impl<T: IoBufMut> IoVectoredBufMut for &'static mut [T] {
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>> {
        self.iter_mut().map(MaybeOwnedMut::Borrowed)
    }
}

impl<T: IoBufMut, const N: usize> IoVectoredBufMut for [T; N] {
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>> {
        self.iter_mut().map(MaybeOwnedMut::Borrowed)
    }
}

impl<T: IoBufMut, #[cfg(feature = "allocator_api")] A: std::alloc::Allocator + 'static>
    IoVectoredBufMut for t_alloc!(Vec, T, A)
{
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>> {
        self.iter_mut().map(MaybeOwnedMut::Borrowed)
    }
}

#[cfg(feature = "arrayvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for arrayvec::ArrayVec<T, N> {
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>> {
        self.iter_mut().map(MaybeOwnedMut::Borrowed)
    }
}

#[cfg(feature = "smallvec")]
impl<T: IoBufMut, const N: usize> IoVectoredBufMut for smallvec::SmallVec<[T; N]>
where
    [T; N]: smallvec::Array<Item = T>,
{
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>> {
        self.iter_mut().map(MaybeOwnedMut::Borrowed)
    }
}

/// A trait for mutable heterogeneous vectored buffers.
pub trait IoVectoredBufMut2: IoVectoredBuf2 + SetBufInit {
    /// Collected [`IoSliceMut`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn io_slices_mut(&mut self) -> Vec<IoSliceMut> {
        self.iter_ioslice_mut().collect()
    }

    /// An iterator over the [`IoSliceMut`]s of the buffers.
    ///
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    /// It is static to provide convenience from writing self-referenced
    /// structure.
    unsafe fn iter_ioslice_mut(&mut self) -> impl Iterator<Item = IoSliceMut>;
}

impl<T: IoBufMut, Rest: IoVectoredBufMut2> IoVectoredBufMut2 for (T, Rest) {
    unsafe fn iter_ioslice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        std::iter::once(self.0.as_io_slice_mut()).chain(self.1.iter_ioslice_mut())
    }
}

impl IoVectoredBufMut2 for () {
    unsafe fn iter_ioslice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        std::iter::empty()
    }
}

impl<T: IoBufMut, Rest: IoVectoredBufMut2> SetBufInit for (T, Rest) {
    unsafe fn set_buf_init(&mut self, len: usize) {
        let buf0_len = min(len, self.0.buf_capacity());

        self.0.set_buf_init(buf0_len);
        self.1.set_buf_init(len - buf0_len);
    }
}

impl SetBufInit for () {
    unsafe fn set_buf_init(&mut self, len: usize) {
        assert_eq!(len, 0, "set_buf_init called with non-zero len on empty buffer");
    }
}

impl<T: IoVectoredBufMut> IoVectoredBufMut2 for T {
    unsafe fn iter_ioslice_mut(&mut self) -> impl Iterator<Item = IoSliceMut> {
        self.iter_buf_mut().map(|mut buf| buf.as_io_slice_mut())
    }
}
