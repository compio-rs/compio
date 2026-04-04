use std::{
    mem::MaybeUninit,
    ops::Not,
    task::{Context, Poll},
    thread,
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

/// A remote view into the task allocation, which is used when the [`Task`]
/// is accessed from a different thread than it's created on.
#[repr(transparent)]
pub(super) struct Remote<'a> {
    header: &'a Header,
}

impl<'a> Remote<'a> {
    pub fn new(header: &'a Header) -> Self {
        Self { header }
    }

    pub fn schedule(&self) {
        let Some(shared) = self.shared() else { return };
        let state = self.state_weak();
        if state.is_scheduled() {
            return;
        }
        let mut notified = false;
        while shared.sync.push(self.header.id).is_err() {
            if !notified && let Some(ref waker) = shared.waker {
                abort_on_panic(|| waker.wake_by_ref());
                notified = true;
            }
            thread::yield_now();
        }
        if !notified && let Some(ref waker) = shared.waker {
            abort_on_panic(|| waker.wake_by_ref());
        }

        self.header.state.set_scheduled::<true>();
    }

    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        instrument!(Level::TRACE, "Remote::poll", id = ?self.header.id);
        let state = self.state();

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
            self.header.state.seting_waker::<true>();

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

        // Task is completed but has no result (shouldn't happen)
        Poll::Ready(None)
    }

    fn state(&self) -> Snapshot {
        self.header.state.load::<Strong>()
    }

    fn state_weak(&self) -> Snapshot {
        self.header.state.load::<Weak>()
    }

    fn shared(&self) -> Option<&Shared> {
        self.state().is_cancelled().not().then(|| {
            // SAFETY: We have checked that the executor is not shutdown, so shared pointer
            // must be present and valid
            let ptr = unsafe { *self.header.shared.get() };
            debug_assert!(!ptr.is_null());
            unsafe { &*ptr }
        })
    }
}
