use std::{marker::PhantomData, mem::MaybeUninit};

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}

/// Reference to a control message.
pub struct CMsgRef<'a>(sys::CMsgRef<'a>);

impl CMsgRef<'_> {
    /// Returns the level of the control message.
    pub fn level(&self) -> i32 {
        self.0.level()
    }

    /// Returns the type of the control message.
    pub fn ty(&self) -> i32 {
        self.0.ty()
    }

    /// Returns the length of the control message.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len() as _
    }

    /// Returns a reference to the data of the control message.
    ///
    /// # Safety
    ///
    /// The data part must be properly aligned and contains an initialized
    /// instance of `T`.
    pub unsafe fn data<T>(&self) -> &T {
        unsafe { self.0.data() }
    }
}

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
            cmsg.map(CMsgRef)
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
    pub fn new(buffer: &'a mut [MaybeUninit<u8>]) -> Self {
        // TODO: optimize zeroing
        buffer.fill(MaybeUninit::new(0));
        Self {
            inner: sys::CMsgIter::new(buffer.as_ptr().cast(), buffer.len()),
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
            self.len += cmsg.set_data(value);

            self.inner.next();
        }

        Some(())
    }
}
