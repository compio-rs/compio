use std::{
    mem::MaybeUninit,
    task::{Context, Poll},
};

use compio_log::{Level, instrument, trace};

use crate::{
    PanicResult, Shared,
    task::{
        Header,
        state::{Snapshot, Strong, Weak},
    },
    util::abort_on_panic,
};

/// A local view into the task allocation, which enables some optimizations.
#[repr(transparent)]
pub(super) struct Local<'a> {
    header: &'a Header,
}

impl<'a> Local<'a> {
    /// # Safety
    ///
    /// The caller must guarantee that this is created on the same thread as the
    /// task allocation.
    #[inline(always)]
    pub unsafe fn new(header: &'a Header) -> Self {
        Self { header }
    }

    pub fn schedule(&self) {
        let Some(shared) = self.shared() else { return };
        // SAFETY: Type invariant
        let queue = unsafe { shared.queue.get_unchecked() };
        while let Some(id) = shared.sync.pop() {
            queue.make_hot(id);
        }
        queue.make_hot(self.header.id);

        if cfg!(feature = "notify-always")
            && let Some(ref waker) = shared.waker
        {
            waker.wake_by_ref()
        }
    }

    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        let state = self.state();
        instrument!(Level::TRACE, "Local::poll", id = ?self.header.id);
        trace!(?state);

        debug_assert!(
            !state.is_completed() || state.has_result() || state.is_cancelled(),
            "Should not poll after the result is taken"
        );

        // Check if the task has completed with a result first
        if state.is_completed() && state.has_result() {
            self.header.state.set_has_result::<Strong, false>();

            let mut res = MaybeUninit::<PanicResult<T>>::uninit();
            let fut = self.header.future_ptr();
            unsafe { (self.header.vtable.take_result)(fut, &raw mut res as _) };

            return Poll::Ready(Some(unsafe { res.assume_init() }));
        }

        // Check cancellation - cancelled tasks without result return None
        if state.is_cancelled() {
            return Poll::Ready(None);
        }

        // Task is not completed yet, set up waker
        if !state.is_completed() {
            let waker = unsafe { &mut *self.header.waker.get() };
            if state.has_waker() {
                if cx.waker().will_wake(unsafe { waker.assume_init_ref() }) {
                    return Poll::Pending;
                }
                unsafe { MaybeUninit::assume_init_drop(waker) };
            }
            waker.write(abort_on_panic(|| cx.waker().clone()));
            self.header.state.set_has_waker::<Weak, true>();

            return Poll::Pending;
        }

        unreachable!("Task is completed but has no result")
    }

    fn state(&self) -> Snapshot {
        self.header.state.load::<Weak>()
    }

    #[inline(always)]
    fn shared(&self) -> Option<&Shared> {
        // SAFETY: We're accessing the pointer locally, the only other party that may
        // change this field is the executor (when it's dropping), which is not
        // happening.
        unsafe { (*self.header.shared.get()).as_ref() }
    }
}
