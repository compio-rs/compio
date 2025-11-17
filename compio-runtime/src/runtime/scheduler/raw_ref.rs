use std::{ops::Deref, ptr::NonNull};

/// A value with ownership.
pub(crate) struct Own<T: ?Sized>(Box<T>);

impl<T> Own<T> {
    /// Creates a new [`Own`].
    pub(crate) fn new(value: T) -> Self {
        Own(Box::new(value))
    }
}

impl<T: ?Sized> Own<T> {
    /// Returns a [`RawRef`] to the owned value.
    pub(crate) fn raw_ref(&self) -> RawRef<T> {
        RawRef(NonNull::from(&*self.0))
    }
}

impl<T: ?Sized> Deref for Own<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A reference to an owned value without lifetime tracking.
pub(crate) struct RawRef<T: ?Sized>(NonNull<T>);

impl<T: ?Sized> RawRef<T> {
    /// Returns a shared reference to the value.
    ///
    /// # Safety
    ///
    /// The caller must ensure the associated [`Own<T>`] outlives the returned
    /// reference.
    pub(crate) const unsafe fn as_ref(&self) -> &T {
        // SAFETY:
        // - The `NonNull` is created from a valid reference in `Own::raw_ref()`.
        // - Only shared reference is returned, so aliasing rules are not violated.
        // - The validity of the returned reference is ensured by the caller.
        unsafe { self.0.as_ref() }
    }
}

/// `Sync` and `Send` implementations follow `&T`.
unsafe impl<T: ?Sized + Sync> Sync for RawRef<T> {}
unsafe impl<T: ?Sized + Sync> Send for RawRef<T> {}

impl<T: ?Sized> Clone for RawRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for RawRef<T> {}
