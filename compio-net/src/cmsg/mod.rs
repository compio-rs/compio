use std::marker::PhantomData;

use compio_buf::{IoBuf, IoBufMut};

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
    pub unsafe fn new<B: IoBuf>(buffer: &'a B) -> Self {
        Self {
            inner: sys::CMsgIter::new(buffer.as_buf_ptr(), buffer.buf_len()),
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
pub struct CMsgBuilder<B> {
    inner: sys::CMsgIter,
    buffer: B,
    len: usize,
}

impl<B> CMsgBuilder<B> {
    /// Finishes building, returns the buffer and the length of the control
    /// message.
    pub fn build(self) -> (B, usize) {
        (self.buffer, self.len)
    }

    /// Try to append a control message entry into the buffer. If the buffer
    /// does not have enough space or is not properly aligned with the value
    /// type, returns `None`.
    ///
    /// # Safety
    ///
    /// TODO: This function may be safe? Given that the buffer is zeroed,
    /// properly aligned and has enough space, safety conditions of all unsafe
    /// functions involved are satisfied, except for `CMSG_*`/`wsa_cmsg_*`, as
    /// their safety are not documented.
    pub unsafe fn try_push<T>(&mut self, level: i32, ty: i32, value: T) -> Option<()> {
        if !self.inner.is_aligned::<T>() || !self.inner.is_space_enough::<T>() {
            return None;
        }

        let mut cmsg = self.inner.current_mut()?;
        cmsg.set_level(level);
        cmsg.set_ty(ty);
        cmsg.set_data(value);

        self.inner.next();
        self.len += sys::space_of::<T>();
        Some(())
    }
}

impl<B: IoBufMut> CMsgBuilder<B> {
    /// Create [`CMsgBuilder`] with the given buffer. The buffer will be zeroed
    /// on creation.
    ///
    /// # Panics
    ///
    /// This function will panic if the buffer is too short or not properly
    /// aligned.
    pub fn new(mut buffer: B) -> Self {
        buffer.as_mut_slice().fill(std::mem::MaybeUninit::zeroed());
        Self {
            inner: sys::CMsgIter::new(buffer.as_buf_mut_ptr(), buffer.buf_len()),
            buffer,
            len: 0,
        }
    }
}
