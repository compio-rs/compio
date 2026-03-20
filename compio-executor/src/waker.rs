use std::{
    mem::ManuallyDrop,
    sync::Arc,
    task::{RawWaker, RawWakerVTable},
};

use crate::queue::Handle;

/// Get the extra data from a waker, if it is a waker created by this module.
///
/// Returns `None` if the waker was not created by compio or the extra type is
/// not `E`.
pub fn get_extra<E: Send + Sync>(waker: &std::task::Waker) -> Option<&E> {
    if waker.vtable() != Waker::<E>::VTABLE {
        return None;
    }

    // SAFETY: We just checked that the vtable matches, so the data must be a
    // `WakerInner`
    Some(&unsafe { &*waker.data().cast::<WakerInner<E>>() }.extra)
}

struct WakerInner<E> {
    handle: Handle,
    extra: E,
}

/// A waker that can hold extra data.
pub struct Waker<E: Send + Sync>(Arc<WakerInner<E>>);

impl<E: Send + Sync> Waker<E> {
    const VTABLE: &'static RawWakerVTable = {
        &RawWakerVTable::new(
            Self::clone_waker,
            Self::wake,
            Self::wake_by_ref,
            Self::drop_waker,
        )
    };

    #[inline(always)]
    unsafe fn clone_waker(waker: *const ()) -> RawWaker {
        unsafe { Arc::increment_strong_count(waker as *const WakerInner<E>) };
        RawWaker::new(waker, Self::VTABLE)
    }

    // Wake by value, moving the Arc into the Wake::wake function
    unsafe fn wake(waker: *const ()) {
        unsafe { Arc::from_raw(waker as *const WakerInner<E>) }
            .handle
            .schedule();
    }

    // Wake by reference, wrap the waker in ManuallyDrop to avoid dropping it
    unsafe fn wake_by_ref(waker: *const ()) {
        ManuallyDrop::new(unsafe { Arc::from_raw(waker as *const WakerInner<E>) })
            .handle
            .schedule();
    }

    // Decrement the reference count of the Arc on drop
    unsafe fn drop_waker(waker: *const ()) {
        unsafe { Arc::decrement_strong_count(waker as *const WakerInner<E>) };
    }

    pub fn new(handle: Handle, extra: E) -> Self {
        Self(Arc::new(WakerInner { handle, extra }))
    }

    /// Convert to [`std::task::Waker`].
    pub fn into_std(self) -> std::task::Waker {
        unsafe { std::task::Waker::new(Arc::into_raw(self.0) as _, Self::VTABLE) }
    }
}

impl<E: Send + Sync> From<Waker<E>> for std::task::Waker {
    fn from(value: Waker<E>) -> Self {
        value.into_std()
    }
}
