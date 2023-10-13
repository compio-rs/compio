use crate::*;

/// An owned buffer iterator for vectored buffers.
/// See [`IoVectoredBuf::owned_iter`].
pub struct OwnedBufIter<T> {
    bufs: T,
    nth: usize,
}

impl<T: IoVectoredBuf> OwnedBufIter<T> {
    pub(crate) fn new(bufs: T, nth: usize) -> Result<Self, T> {
        if bufs.as_dyn_bufs().nth(nth).is_none() {
            Err(bufs)
        } else {
            Ok(Self { bufs, nth })
        }
    }

    /// Get the next buffer. Will return Err with the inner buffer if it reaches
    /// the end.
    pub fn next(self) -> Result<Self, T> {
        Self::new(self.bufs, self.nth + 1)
    }

    fn get_slice(&self) -> &dyn IoBuf {
        self.bufs
            .as_dyn_bufs()
            .nth(self.nth)
            .expect("the nth buf should exist")
    }
}

impl<T: IoVectoredBufMut> OwnedBufIter<T> {
    fn get_slice_mut(&mut self) -> &mut dyn IoBufMut {
        self.bufs
            .as_dyn_mut_bufs()
            .nth(self.nth)
            .expect("the nth buf should exist")
    }
}

unsafe impl<T: IoVectoredBuf> IoBuf for OwnedBufIter<T> {
    fn as_buf_ptr(&self) -> *const u8 {
        self.get_slice().as_buf_ptr()
    }

    fn buf_len(&self) -> usize {
        self.get_slice().buf_len()
    }

    fn buf_capacity(&self) -> usize {
        self.get_slice().buf_capacity()
    }
}

unsafe impl<T: IoVectoredBufMut> IoBufMut for OwnedBufIter<T> {
    fn as_buf_mut_ptr(&mut self) -> *mut u8 {
        self.get_slice_mut().as_buf_mut_ptr()
    }
}

impl<T: IoVectoredBufMut> SetBufInit for OwnedBufIter<T> {
    unsafe fn set_buf_init(&mut self, len: usize) {
        self.get_slice_mut().set_buf_init(len)
    }
}

impl<T> IntoInner for OwnedBufIter<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.bufs
    }
}
