use std::pin::Pin;

use compio_buf::{
    Indexable, IndexableMut, IndexedIter, IntoInner, IoBuf, IoBufMut, IoVectoredBuf,
    IoVectoredBufMut, MaybeOwned, MaybeOwnedMut, SetBufInit,
};

pub struct VectoredWrap<T> {
    buffers: Pin<Box<T>>,
    wraps: Vec<BufWrap>,
    vec_off: usize,
}

impl<T: IoVectoredBuf> VectoredWrap<T> {
    pub fn new(buffers: T) -> Self {
        let buffers = Box::pin(buffers);
        let wraps = buffers.iter_buf().map(|buf| BufWrap::new(&*buf)).collect();
        Self {
            buffers,
            wraps,
            vec_off: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.wraps.iter().map(|buf| buf.len).sum()
    }

    pub fn capacity(&self) -> usize {
        self.wraps.iter().map(|buf| buf.capacity).sum()
    }
}

impl<T: IoVectoredBuf + 'static> IoVectoredBuf for VectoredWrap<T> {
    type Buf = BufWrap;
    type OwnedIter = IndexedIter<Self>;

    fn iter_buf(&self) -> impl Iterator<Item = MaybeOwned<'_, Self::Buf>> {
        self.wraps
            .iter()
            .skip(self.vec_off)
            .map(MaybeOwned::Borrowed)
    }

    fn owned_iter(self) -> Result<Self::OwnedIter, Self>
    where
        Self: Sized,
    {
        IndexedIter::new(self)
    }
}

impl<T: IoVectoredBufMut + 'static> IoVectoredBufMut for VectoredWrap<T> {
    fn iter_buf_mut(&mut self) -> impl Iterator<Item = MaybeOwnedMut<'_, Self::Buf>> {
        self.wraps
            .iter_mut()
            .skip(self.vec_off)
            .map(MaybeOwnedMut::Borrowed)
    }
}

impl<T: SetBufInit> SetBufInit for VectoredWrap<T> {
    unsafe fn set_buf_init(&mut self, mut len: usize) {
        self.buffers.as_mut().get_unchecked_mut().set_buf_init(len);
        self.vec_off = 0;
        for buf in self.wraps.iter_mut().skip(self.vec_off) {
            let capacity = (*buf).buf_capacity();
            let buf_new_len = len.min(capacity);
            buf.set_buf_init(buf_new_len);
            *buf = buf.offset(buf_new_len);
            if len >= capacity {
                len -= capacity;
            } else {
                break;
            }
            self.vec_off += 1;
        }
    }
}

impl<T> Indexable for VectoredWrap<T> {
    type Output = BufWrap;

    fn index(&self, n: usize) -> Option<&Self::Output> {
        self.wraps.get(n + self.vec_off)
    }
}

impl<T> IndexableMut for VectoredWrap<T> {
    fn index_mut(&mut self, n: usize) -> Option<&mut Self::Output> {
        self.wraps.get_mut(n + self.vec_off)
    }
}

impl<T> IntoInner for VectoredWrap<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        // Safety: no pointers still maintaining
        *unsafe { Pin::into_inner_unchecked(self.buffers) }
    }
}

pub struct BufWrap {
    ptr: *mut u8,
    len: usize,
    capacity: usize,
}

impl BufWrap {
    fn new<T: IoBuf>(buf: &T) -> Self {
        Self {
            ptr: buf.as_buf_ptr().cast_mut(),
            len: buf.buf_len(),
            capacity: buf.buf_capacity(),
        }
    }

    fn offset(&self, off: usize) -> Self {
        Self {
            ptr: unsafe { self.ptr.add(off) },
            len: self.len.saturating_sub(off),
            capacity: self.capacity.saturating_sub(off),
        }
    }
}

unsafe impl IoBuf for BufWrap {
    fn as_buf_ptr(&self) -> *const u8 {
        self.ptr.cast_const()
    }

    fn buf_len(&self) -> usize {
        self.len
    }

    fn buf_capacity(&self) -> usize {
        self.capacity
    }
}

unsafe impl IoBufMut for BufWrap {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
}

impl SetBufInit for BufWrap {
    unsafe fn set_buf_init(&mut self, len: usize) {
        debug_assert!(len <= self.capacity, "{} > {}", len, self.capacity);
        self.len = self.len.max(len);
    }
}
