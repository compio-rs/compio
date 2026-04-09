use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::Ordering::*,
    task::{Context, Poll},
};

use compio_log::{instrument, trace};

use crate::{
    PanicResult,
    task::{
        Header,
        state::{Snapshot, Strong, Weak},
    },
};

/// A local view into the task allocation, which enables some optimizations.
#[repr(transparent)]
pub(super) struct Local<'a> {
    ptr: NonNull<Header>,
    marker: PhantomData<&'a Header>,
}

impl<'a> Local<'a> {
    /// # Safety
    ///
    /// The caller must guarantee that this is created on the same thread as the
    /// task allocation.
    #[inline(always)]
    pub unsafe fn new(ptr: NonNull<Header>) -> Self {
        Self {
            ptr,
            marker: PhantomData,
        }
    }

    pub fn schedule(&self) {
        instrument!(compio_log::Level::TRACE, "Local::schedule", id = ?self.header().id);

        // Load shared pointer atomically - it may be null if cancelled
        // `Relaxed` is fine here since we're on the same thread as `Executor`, there's
        // no way to schedule as task while it's being dropped
        let Some(shared) = (unsafe { self.header().shared.load(Relaxed).as_ref() }) else {
            trace!("Executor dropped");
            return;
        };

        trace!("Not dropped");

        // SAFETY: Type invariant
        let queue = unsafe { shared.queue.get_unchecked() };
        while let Some(id) = shared.sync.pop() {
            trace!(?id, "Scheduling");
            queue.make_hot(id);
        }

        trace!(id = ?self.header().id, "Scheduling self");

        queue.make_hot(self.header().id);

        if cfg!(feature = "notify-always")
            && let Some(ref waker) = shared.waker
        {
            waker.wake_by_ref()
        }
    }

    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        let state = self.state();
        trace!(?state);

        debug_assert!(state.has_result() || !state.is_completed() || state.is_cancelled());

        // The task is completed, take the result
        if state.has_result() {
            self.header().state.set_has_result::<Strong, false>();

            let mut res = MaybeUninit::<PanicResult<T>>::uninit();
            let target = NonNull::from_mut(&mut res).cast();
            unsafe { (self.header().vtable.take_result)(self.ptr, target) };

            return Poll::Ready(Some(unsafe { res.assume_init() }));
        }

        // The task is cancelled without result, return None
        if state.is_cancelled() {
            return Poll::Ready(None);
        }

        // Task is not completed yet, set up waker
        if !state.is_completed() {
            return self.header().waker.with_mut(|waker| {
                crate::panic_guard!();
                let waker = unsafe { &mut *waker };
                if state.has_waker() {
                    if cx.waker().will_wake(unsafe { waker.assume_init_ref() }) {
                        return Poll::Pending;
                    }
                    unsafe { waker.assume_init_drop() };
                }
                waker.write(cx.waker().clone());
                self.header().state.set_has_waker::<Weak, true>();

                Poll::Pending
            });
        }

        unreachable!("Task is completed but has no result")
    }

    fn header(&self) -> &Header {
        unsafe { self.ptr.as_ref() }
    }

    fn state(&self) -> Snapshot {
        self.header().state.load::<Weak>()
    }
}
