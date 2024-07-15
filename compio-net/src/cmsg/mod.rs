use std::marker::PhantomData;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}

pub use sys::CMsgRef;

/// An iterator for control messages.
pub struct CMsgIter<'a> {
    inner: sys::CMsgIter,
    _p: PhantomData<&'a ()>,
}

impl<'a> CMsgIter<'a> {
    /// Create [`CMsgIter`] with the given buffer.
    ///
    /// # Panics
    ///
    /// This function will panic if the buffer is too short or not properly
    /// aligned.
    ///
    /// # Safety
    ///
    /// The buffer should contain valid control messages.
    pub unsafe fn new(buffer: &'a [u8]) -> Self {
        Self {
            inner: sys::CMsgIter::new(buffer.as_ptr(), buffer.len()),
            _p: PhantomData,
        }
    }
}

impl<'a> Iterator for CMsgIter<'a> {
    type Item = CMsgRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let cmsg = self.inner.current();
            self.inner.next();
            cmsg
        }
    }
}

/// Helper to construct control message.
pub struct CMsgBuilder<'a> {
    inner: sys::CMsgIter,
    len: usize,
    _p: PhantomData<&'a mut ()>,
}

impl<'a> CMsgBuilder<'a> {
    /// Create [`CMsgBuilder`] with the given buffer. The buffer will be zeroed
    /// on creation.
    ///
    /// # Panics
    ///
    /// This function will panic if the buffer is too short or not properly
    /// aligned.
    pub fn new(buffer: &'a mut [u8]) -> Self {
        buffer.fill(0);
        Self {
            inner: sys::CMsgIter::new(buffer.as_ptr(), buffer.len()),
            len: 0,
            _p: PhantomData,
        }
    }

    /// Finishes building, returns length of the control message.
    pub fn finish(self) -> usize {
        self.len
    }

    /// Try to append a control message entry into the buffer. If the buffer
    /// does not have enough space or is not properly aligned with the value
    /// type, returns `None`.
    pub fn try_push<T>(&mut self, level: i32, ty: i32, value: T) -> Option<()> {
        if !self.inner.is_aligned::<T>() || !self.inner.is_space_enough::<T>() {
            return None;
        }

        // SAFETY: the buffer is zeroed and the pointer is valid and aligned
        unsafe {
            let mut cmsg = self.inner.current_mut()?;
            cmsg.set_level(level);
            cmsg.set_ty(ty);
            cmsg.set_data(value);

            self.inner.next();
            self.len += sys::space_of::<T>();
        }

        Some(())
    }
}
