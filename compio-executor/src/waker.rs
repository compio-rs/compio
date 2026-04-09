use std::{
    mem::ManuallyDrop,
    task::{RawWaker, RawWakerVTable, Waker},
};

use crate::task::Task;

impl Task {
    const VTABLE: &'static RawWakerVTable = {
        &RawWakerVTable::new(
            Self::clone_waker,
            Self::wake,
            Self::wake_by_ref,
            Self::drop_waker,
        )
    };

    #[inline(always)]
    unsafe fn clone_waker(ptr: *const ()) -> RawWaker {
        unsafe { Task::increment_count(ptr) };
        RawWaker::new(ptr, Self::VTABLE)
    }

    unsafe fn wake(ptr: *const ()) {
        unsafe { Task::from_raw(ptr) }.schedule();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        ManuallyDrop::new(unsafe { Task::from_raw(ptr) }).schedule();
    }

    unsafe fn drop_waker(ptr: *const ()) {
        drop(unsafe { Task::from_raw(ptr) });
    }

    /// Get a reference to [`Waker`] and run a closure on it.
    ///
    /// This will not increase the reference counter and is very cheap to call.
    pub fn with_waker<R, F: FnOnce(&Waker) -> R>(&self, f: F) -> R {
        let raw = RawWaker::new(self.as_raw(), Self::VTABLE);
        f(&ManuallyDrop::new(unsafe { Waker::from_raw(raw) }))
    }
}
